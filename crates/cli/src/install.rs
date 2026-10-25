//! Per-user installation. On macOS a LaunchAgent keeps the daemon running; on Windows the
//! desktop app starts it (the installer registers the app to start at sign-in).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clashofmeat_core::{ConfigWriter, Env, WriteOutcome};
use clashofmeat_daemon::{config, wiring};

use crate::{client, migrate};

const LABEL: &str = "com.clashofmeat.daemon";

pub fn install(env: &Env, app: &str, no_launch_agent: bool) -> Result<()> {
    migrate::adopt_legacy_config(env)?;
    let origin = config::app_origin(app)?;
    config::update_config(env, |config| config.app_url = Some(origin.clone()))?;
    migrate::run(env, &origin)?;

    if !no_launch_agent && cfg!(target_os = "macos") {
        let path = launch_agent_path(env);
        let exe = std::env::current_exe()?.canonicalize()?;
        let log_dir = config::home(env).join("logs");
        fs::create_dir_all(&log_dir)?;
        let writer = ConfigWriter::new(config::home(env).join("backup"));
        writer.write(&path, plist(env, &exe).as_bytes())?;
        let target = format!("gui/{}/{}", uid()?, LABEL);
        let _ = launchctl(env, &["bootout", &target]);
        if launchctl(
            env,
            &[
                "bootstrap",
                &format!("gui/{}", uid()?),
                &path.to_string_lossy(),
            ],
        )
        .is_err()
        {
            launchctl(env, &["load", "-w", &path.to_string_lossy()])?;
        }
        launchctl(env, &["kickstart", &target])?;
        println!("Installed LaunchAgent {}.", path.display());
        let deadline = Instant::now() + Duration::from_secs(10);
        while client::Client::connect(env).is_err() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
        }
        if client::Client::connect(env).is_err() {
            eprintln!(
                "Daemon socket is not ready yet; check {}.",
                log_dir.join("daemon.log").display()
            );
        }
    }

    let writer = ConfigWriter::new(config::home(env).join("backup"));
    match wiring::wire_cursor(env, &writer)? {
        WriteOutcome::Written { backup } => println!(
            "Wired Cursor hooks in {} (backup {}).",
            env.home.join(".cursor/hooks.json").display(),
            backup.display()
        ),
        WriteOutcome::Unchanged if env.home.join(".cursor").is_dir() => {
            println!("Cursor hooks already wired.")
        }
        WriteOutcome::Unchanged => {}
    }

    if cfg!(target_os = "macos")
        && env.vars.get("CLASHOFMEAT_NO_OPEN").map(String::as_str) != Some("1")
    {
        if let Some(app_path) = running_app_path()? {
            let _ = Command::new("/usr/bin/open")
                .arg("-a")
                .arg(app_path)
                .status();
        }
    }
    Ok(())
}

pub fn uninstall(env: &Env, keep_data: bool) -> Result<()> {
    // Before anything else, while the token and the app address are still here.
    if clashofmeat_daemon::sync::revoke_installation(env, env!("CARGO_PKG_VERSION"))? {
        println!("Disconnected this computer from Clash of Meat.");
    }
    if cfg!(target_os = "macos") {
        remove_mac_install(env)?;
    } else {
        wiring::unwire_cursor(env, &ConfigWriter::new(config::home(env).join("backup")))?;
    }
    if !keep_data {
        for name in ["desktop", "hero", "spool"] {
            let path = config::home(env).join(name);
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else if path.exists() {
                fs::remove_file(path)?;
            }
        }
        let socket = config::home(env).join("daemon.sock");
        if socket.exists() {
            fs::remove_file(socket)?;
        }
    }
    Ok(())
}

fn remove_mac_install(env: &Env) -> Result<()> {
    let path = launch_agent_path(env);
    let target = format!("gui/{}/{}", uid()?, LABEL);
    if path.exists() {
        let _ = launchctl(env, &["bootout", &target]);
        fs::remove_file(&path)?;
        println!("Removed LaunchAgent {}.", path.display());
    }
    wiring::unwire_cursor(env, &ConfigWriter::new(config::home(env).join("backup")))?;
    let link = env.home.join(".local/bin/clashofmeat");
    if let Ok(target) = fs::read_link(&link) {
        if target.to_string_lossy().contains("Clash of Meat.app/") {
            fs::remove_file(&link)?;
            println!("Removed {}.", link.display());
        }
    }
    let installed_app = env.home.join("Applications/Clash of Meat.app");
    if running_app_path()?.as_deref() == Some(installed_app.as_path()) {
        fs::remove_dir_all(&installed_app)?;
        println!("Removed {}.", installed_app.display());
    } else if installed_app.exists() {
        println!("Remove {} manually when ready.", installed_app.display());
    }
    Ok(())
}

pub(crate) fn launch_agent_path(env: &Env) -> PathBuf {
    env.vars
        .get("CLASHOFMEAT_LAUNCH_AGENTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home.join("Library/LaunchAgents"))
        .join(format!("{LABEL}.plist"))
}

pub(crate) fn launchctl(env: &Env, args: &[&str]) -> Result<()> {
    let program = env
        .vars
        .get("CLASHOFMEAT_LAUNCHCTL")
        .map(String::as_str)
        .unwrap_or("/bin/launchctl");
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("launchctl {} failed with {status}", args.join(" "));
    }
    Ok(())
}

pub(crate) fn uid() -> Result<String> {
    let output = Command::new("/usr/bin/id").arg("-u").output()?;
    if !output.status.success() {
        bail!("could not determine user ID");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn running_app_path() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let Some(app) = exe.ancestors().find(|parent| {
        parent
            .file_name()
            .is_some_and(|name| name == "Clash of Meat.app")
    }) else {
        return Ok(None);
    };
    Ok(Some(app.to_path_buf()))
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn plist(env: &Env, exe: &Path) -> String {
    let log = config::home(env).join("logs/daemon.log");
    // A LaunchAgent doesn't see the shell's environment, so agent folders moved with these
    // variables are passed on as they were when `clashofmeat install` ran.
    let agent_homes: String = ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CLI_HOME"]
        .iter()
        .filter_map(|name| {
            let value = env.vars.get(*name).filter(|value| !value.is_empty())?;
            Some(format!("<key>{name}</key><string>{}</string>", xml(value)))
        })
        .collect();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>daemon</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ProcessType</key><string>Background</string>\n<key>StandardOutPath</key><string>{}</string>\n<key>StandardErrorPath</key><string>{}</string>\n<key>EnvironmentVariables</key><dict><key>PATH</key><string>/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string><key>HOME</key><string>{}</string>{agent_homes}</dict>\n</dict></plist>\n", xml(&exe.to_string_lossy()), xml(&log.to_string_lossy()), xml(&log.to_string_lossy()), xml(&env.home.to_string_lossy()))
}
