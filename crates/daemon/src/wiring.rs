//! Cursor hook installation through the shared atomic configuration writer.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clashofmeat_core::{ConfigWriter, Env, RestoreOutcome, WriteOutcome};
use serde_json::{json, Value};

const EVENTS: [&str; 4] = [
    "beforeSubmitPrompt",
    "postToolUse",
    "afterAgentResponse",
    "stop",
];

/// The command Cursor executes for a hook event.
pub fn cursor_hook_command(env: &Env) -> String {
    let bin_dir = env
        .vars
        .get("CLASHOFMEAT_BIN_DIR")
        .map_or_else(|| env.home.join(".local/bin"), PathBuf::from);
    let bin_dir = if bin_dir.is_absolute() {
        bin_dir
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(bin_dir)
    };
    let link = bin_dir.join("clashofmeat");
    let executable = if link.exists() {
        link
    } else {
        std::env::current_exe().unwrap_or(link)
    };
    format!("{} cursor-hook", shell_quote(&executable))
}

fn shell_quote(path: &Path) -> String {
    let string = path.to_string_lossy();
    format!("'{}'", string.replace('\'', "'\\''"))
}

fn hooks_path(env: &Env) -> PathBuf {
    env.home.join(".cursor/hooks.json")
}

fn is_ours(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            (command.contains("clashofmeat") || command.contains("guildbyte"))
                && command.contains("cursor-hook")
        })
}

fn remove_ours(document: &mut Value) -> bool {
    let Some(hooks) = document.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut changed = false;
    hooks.retain(|_, entries| {
        if let Some(entries) = entries.as_array_mut() {
            let old = entries.len();
            entries.retain(|entry| !is_ours(entry));
            changed |= old != entries.len();
            !entries.is_empty()
        } else {
            true
        }
    });
    changed
}

fn merge(document: &mut Value, command: &str) -> bool {
    let original = document.clone();
    if !document.is_object() {
        *document = json!({});
    }
    let object = document.as_object_mut().expect("object above");
    object.entry("version").or_insert(json!(1));
    if !object.get("hooks").is_some_and(Value::is_object) {
        object.insert("hooks".into(), json!({}));
    }
    let hooks = object
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .expect("object above");
    for event in EVENTS {
        let entries = hooks.entry(event).or_insert_with(|| json!([]));
        if !entries.is_array() {
            *entries = json!([]);
        }
        let entries = entries.as_array_mut().expect("array above");
        entries.retain(|entry| !is_ours(entry));
        entries.push(json!({"command":command,"timeout":5}));
    }
    original != *document
}

/// Merges our four hooks when Cursor is installed, preserving unrelated hooks.
pub fn wire_cursor(env: &Env, writer: &ConfigWriter) -> Result<WriteOutcome> {
    if !env.home.join(".cursor").is_dir() {
        return Ok(WriteOutcome::Unchanged);
    }
    let command = cursor_hook_command(env);
    Ok(writer.update_json(&hooks_path(env), |document| merge(document, &command))?)
}

/// Removes installed hooks and restores the original bytes if no one else edited the file.
pub fn unwire_cursor(env: &Env, writer: &ConfigWriter) -> Result<()> {
    let path = hooks_path(env);
    if !path.exists() {
        return Ok(());
    }
    if let Some(original) = backup_original(env, &path)? {
        let current = fs::read(&path)?;
        let mut expected = original
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_else(|| json!({}));
        merge(&mut expected, &cursor_hook_command(env));
        let mut expected = serde_json::to_vec_pretty(&expected)?;
        expected.push(b'\n');
        if current == expected && writer.restore(&path)? == RestoreOutcome::Restored {
            return Ok(());
        }
    }
    writer.update_json(&path, remove_ours)?;
    Ok(())
}

fn backup_original(env: &Env, target: &Path) -> Result<Option<Option<Vec<u8>>>> {
    let root = crate::config::home(env).join("backup");
    let Ok(backups) = fs::read_dir(root) else {
        return Ok(None);
    };
    let target = fs::canonicalize(target)?;
    for entry in backups.flatten() {
        let backup = entry.path();
        if fs::read(backup.join("path.txt")).ok().as_deref()
            == Some(target.to_string_lossy().as_bytes())
        {
            return Ok(Some(fs::read(backup.join("original")).ok()));
        }
    }
    Ok(None)
}

/// Current Cursor hook state for the settings window.
pub fn cursor_wiring_state(env: &Env) -> &'static str {
    if !env.home.join(".cursor").is_dir() {
        return "absent";
    }
    let Ok(bytes) = fs::read(hooks_path(env)) else {
        return "missing";
    };
    let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
        return "missing";
    };
    let command = cursor_hook_command(env);
    if EVENTS.iter().all(|event| {
        document["hooks"][event]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["command"] == command))
    }) {
        "ok"
    } else {
        "missing"
    }
}
