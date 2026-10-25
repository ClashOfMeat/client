//! Idempotent import of the Node collector's pairing and reader offsets.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use clashofmeat_core::{reader, Env, FileCursor, Store};
use clashofmeat_daemon::config;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::install;

pub fn adopt_legacy_config(env: &Env) -> Result<()> {
    let legacy = env.home.join(".guildbyte");
    if !legacy.is_dir() || config::home(env).join("config.json").exists() {
        return Ok(());
    }
    let path = legacy.join("config.json");
    if let Ok(bytes) = fs::read(&path) {
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let Some(origin) = value.get("appUrl").and_then(serde_json::Value::as_str) {
            let origin = config::app_origin(origin)?;
            config::update_config(env, |config| config.app_url = Some(origin))?;
            println!(
                "Imported app URL from {}. You can remove {} when ready.",
                path.display(),
                legacy.display()
            );
        }
    }
    Ok(())
}

pub fn run(env: &Env, origin: &str) -> Result<()> {
    let hash = hex::encode(Sha256::digest(origin.as_bytes()));
    let current_path = config::home(env).join(&hash[..16]).join("collector.sqlite");
    let legacy_path = env
        .home
        .join(".guildbyte")
        .join(&hash[..16])
        .join("collector.sqlite");
    let old_path = if current_path.is_file() {
        current_path
    } else {
        legacy_path
    };
    if old_path.is_file() {
        let old = Connection::open_with_flags(&old_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut store = Store::open(config::store_path(env))?;
        if clashofmeat_daemon::sync::installation_token(&mut store, Some(origin))?.is_none()
            && table_exists(&old, "accounts")?
        {
            let account: Option<(String, String, Option<String>)> = old.query_row(
                    "SELECT id, token, installation_id FROM accounts WHERE plan='collector' AND token IS NOT NULL ORDER BY rowid LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ).optional()?;
            if let Some((account_id, token, installation_id)) = account {
                let count = migrate_offsets(env, &old, &mut store)?;
                if let Some(installation_id) = installation_id {
                    store.set_meta("installation.id", &installation_id)?;
                }
                if table_exists(&old, "metadata")? {
                    let paired: Option<String> = old
                        .query_row(
                            "SELECT value FROM metadata WHERE key=?1",
                            [format!("paired-at:{account_id}")],
                            |row| row.get(0),
                        )
                        .optional()?;
                    if let Some(paired) = paired {
                        store.set_meta("paired_at", &paired)?;
                    }
                }
                store.set_meta("installation.token", &token)?;
                store.set_meta("installation.origin", origin)?;
                println!(
                    "Imported collector pairing and {count} file offsets from {}.",
                    old_path.display()
                );
            }
        }
    }
    // The Node collector's LaunchAgent, under its name before and after the rename.
    for label in ["com.clashofmeat.collector", "com.guildbyte.collector"] {
        let old_agent = install::launch_agent_path(env).with_file_name(format!("{label}.plist"));
        if old_agent.exists() {
            let target = format!("gui/{}/{label}", install::uid()?);
            let _ = install::launchctl(env, &["bootout", &target]);
            fs::remove_file(&old_agent)?;
            println!("Removed old collector LaunchAgent {}.", old_agent.display());
        }
    }
    // On Windows the Node collector ran as a scheduled task.
    #[cfg(windows)]
    for task in ["ClashOfMeatCollector", "GuildbyteCollector"] {
        let removed = std::process::Command::new("schtasks")
            .args(["/Delete", "/F", "/TN", task])
            .output()
            .is_ok_and(|output| output.status.success());
        if removed {
            println!("Removed old collector scheduled task {task}.");
        }
    }
    let home = config::home(env);
    if home.join("app").exists() || home.join("bin").exists() {
        println!(
            "Old collector files in {}/app and {}/bin are no longer used.",
            home.display(),
            home.display()
        );
    }
    let plugins = env.home.join(".claude/plugins");
    if legacy_plugin_installed(&plugins)? {
        println!(
            "Remove the old Claude plugin with: claude plugin uninstall clashofmeat@clashofmeat"
        );
    }
    Ok(())
}

fn table_exists(db: &Connection, table: &str) -> Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn migrate_offsets(env: &Env, old: &Connection, store: &mut Store) -> Result<usize> {
    let mut count = 0;
    for table in ["provider_files", "files"] {
        if !table_exists(old, table)? {
            continue;
        }
        let mut statement = old.prepare(&format!("SELECT path, offset FROM {table}"))?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (path, offset) = row?;
            if offset < 0 {
                continue;
            }
            if let Some((provider, source, file_key, cursor)) =
                map_cursor(env, Path::new(&path), offset as u64)
            {
                if store.cursor(&provider, source, &file_key)?.is_none() {
                    store.put_cursor(&provider, source, &file_key, &cursor)?;
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn map_cursor(env: &Env, path: &Path, offset: u64) -> Option<(String, usize, String, FileCursor)> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    for provider in clashofmeat_providers::all() {
        let manifest = provider.manifest();
        for (source, spec) in manifest.sources.iter().enumerate() {
            let root = env.resolve(spec.root);
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            if !reader::depth_ok(relative, 4) || !reader::glob_matches(spec.glob, relative) {
                continue;
            }
            let modified = metadata.modified().ok()?;
            let mtime_ms = match modified.duration_since(UNIX_EPOCH) {
                Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
                Err(error) => -i64::try_from(error.duration().as_millis()).unwrap_or(i64::MAX),
            };
            #[cfg(unix)]
            let inode = {
                use std::os::unix::fs::MetadataExt;
                metadata.ino()
            };
            #[cfg(not(unix))]
            let inode = 0;
            return Some((
                manifest.id.to_owned(),
                source,
                relative.to_string_lossy().into_owned(),
                FileCursor {
                    inode,
                    offset,
                    len: metadata.len(),
                    mtime_ms,
                    ..FileCursor::default()
                },
            ));
        }
    }
    None
}

fn legacy_plugin_installed(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(path)? {
        let name = entry?.file_name().to_string_lossy().to_ascii_lowercase();
        if name.contains("clashofmeat") || name.contains("guildbyte") {
            return Ok(true);
        }
    }
    let installed = PathBuf::from(path).join("installed_plugins.json");
    Ok(fs::read_to_string(installed)
        .is_ok_and(|text| text.contains("clashofmeat") || text.contains("guildbyte")))
}
