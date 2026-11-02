//! Linux installation. A systemd user service keeps the daemon running and a daily timer runs
//! `clashofmeat update`. Without a systemd user session (some containers and WSL setups) the
//! daemon is started in the background for this session only.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use clashofmeat_core::{ConfigWriter, Env};
use clashofmeat_daemon::config;

use crate::client;

pub(crate) const SERVICE: &str = "clashofmeat.service";
const UPDATE_SERVICE: &str = "clashofmeat-update.service";
const UPDATE_TIMER: &str = "clashofmeat-update.timer";
/// The Node collector's units, under its name before and after the rename.
const LEGACY_UNITS: [&str; 2] = ["clashofmeat-collector", "guildbyte-collector"];
/// Agent folders moved with these variables are passed on to the service as they were at install.
const PASSED_VARS: [&str; 4] = ["PATH", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CLI_HOME"];

/// Where install.sh puts the CLI: `${XDG_DATA_HOME:-~/.local/share}/clashofmeat/bin/clashofmeat`.
pub(crate) fn installed_exe(env: &Env) -> PathBuf {
    env.vars
        .get("XDG_DATA_HOME")
        .filter(|value| value.starts_with('/'))
        .map_or_else(|| env.home.join(".local/share"), PathBuf::from)
        .join("clashofmeat/bin/clashofmeat")
}

pub(crate) fn unit_dir(env: &Env) -> PathBuf {
    if let Some(dir) = env.vars.get("CLASHOFMEAT_SYSTEMD_DIR") {
        return PathBuf::from(dir);
    }
    env.vars
        .get("XDG_CONFIG_HOME")
        .filter(|value| value.starts_with('/'))
        .map_or_else(|| env.home.join(".config"), PathBuf::from)
        .join("systemd/user")
}

fn systemctl_command(env: &Env, args: &[&str]) -> Command {
    let program = env
        .vars
        .get("CLASHOFMEAT_SYSTEMCTL")
        .map(String::as_str)
        .unwrap_or("systemctl");
    let mut command = Command::new(program);
    command.arg("--user").args(args).stdin(Stdio::null());
    command
}

pub(crate) fn systemctl(env: &Env, args: &[&str]) -> Result<()> {
    let output = systemctl_command(env, args)
        .output()
        .context("running systemctl")?;
    if !output.status.success() {
        bail!(
            "systemctl --user {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Whether a systemd user manager answers for this user.
pub(crate) fn has_user_systemd(env: &Env) -> bool {
    systemctl_command(env, &["show-environment"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Installs and (re)starts the service. Returns false when there is no systemd user session and
/// the daemon was started for this session only.
pub fn install_service(env: &Env, exe: &Path) -> Result<bool> {
    let log = config::home(env).join("logs/daemon.log");
    if !has_user_systemd(env) {
        start_detached(env, exe, &log)?;
        println!(
            "No systemd user session here, so Clash of Meat runs until you log out or restart. \
             Run `clashofmeat daemon` from your startup scripts to keep it running."
        );
        return Ok(false);
    }
    let dir = unit_dir(env);
    fs::create_dir_all(&dir)?;
    let writer = ConfigWriter::new(config::home(env).join("backup"));
    writer.write(&dir.join(SERVICE), service_unit(env, exe, &log)?.as_bytes())?;
    writer.write(&dir.join(UPDATE_SERVICE), update_unit(exe)?.as_bytes())?;
    writer.write(&dir.join(UPDATE_TIMER), TIMER_UNIT.as_bytes())?;
    systemctl(env, &["daemon-reload"])?;
    systemctl(env, &["enable", SERVICE, UPDATE_TIMER])?;
    // restart, not start: a reinstall must run the new binary.
    systemctl(env, &["restart", SERVICE])?;
    systemctl(env, &["start", UPDATE_TIMER])?;
    println!(
        "Installed systemd user service {}.",
        dir.join(SERVICE).display()
    );
    if env.vars.contains_key("SSH_CONNECTION") && !lingering(env) {
        println!(
            "To keep collecting after you log out of this SSH session, run: loginctl enable-linger"
        );
    }
    Ok(true)
}

/// Stops and removes the service and the update timer.
pub fn remove_service(env: &Env) -> Result<()> {
    let dir = unit_dir(env);
    let units = [SERVICE, UPDATE_SERVICE, UPDATE_TIMER];
    if units.iter().any(|unit| dir.join(unit).exists()) {
        let _ = systemctl(env, &["disable", "--now", UPDATE_TIMER, SERVICE]);
        for unit in units {
            let path = dir.join(unit);
            if path.exists() {
                fs::remove_file(&path)?;
            }
        }
        let _ = systemctl(env, &["daemon-reload"]);
        println!(
            "Removed systemd user service {}.",
            dir.join(SERVICE).display()
        );
    }
    Ok(())
}

/// Removes what install.sh put in place: the `~/.local/bin` link and the installed CLI.
pub fn remove_files(env: &Env) -> Result<()> {
    let installed = installed_exe(env);
    let link = env.home.join(".local/bin/clashofmeat");
    if fs::read_link(&link).is_ok_and(|target| target == installed) {
        fs::remove_file(&link)?;
        println!("Removed {}.", link.display());
    }
    if let Some(dir) = installed.parent().and_then(Path::parent) {
        if installed.exists() {
            fs::remove_dir_all(dir)?;
            println!("Removed {}.", dir.display());
        }
    }
    Ok(())
}

/// Stops and removes the Node collector's systemd units.
pub fn remove_legacy_units(env: &Env) -> Result<()> {
    let dir = unit_dir(env);
    for name in LEGACY_UNITS {
        let files = [format!("{name}.timer"), format!("{name}.service")];
        if !files.iter().any(|file| dir.join(file).exists()) {
            continue;
        }
        let _ = systemctl(env, &["disable", "--now", &files[0], &files[1]]);
        for file in &files {
            let path = dir.join(file);
            if path.exists() {
                fs::remove_file(&path)?;
            }
        }
        let _ = systemctl(env, &["daemon-reload"]);
        println!("Removed old collector systemd units {name}.");
    }
    Ok(())
}

/// Restarts the service so it runs a newly installed binary; false when there is no service.
pub fn restart_service(env: &Env) -> Result<bool> {
    if !unit_dir(env).join(SERVICE).exists() || !has_user_systemd(env) {
        return Ok(false);
    }
    systemctl(env, &["restart", SERVICE])?;
    Ok(true)
}

fn lingering(env: &Env) -> bool {
    let Some(user) = env.vars.get("USER") else {
        return false;
    };
    Command::new("loginctl")
        .args(["show-user", user, "--property=Linger", "--value"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "yes")
}

/// Starts `clashofmeat daemon` in its own session, so closing the terminal does not stop it.
fn start_detached(env: &Env, exe: &Path, log: &Path) -> Result<()> {
    if client::Client::connect(env).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(log.parent().expect("log has a parent"))?;
    let out = fs::OpenOptions::new().create(true).append(true).open(log)?;
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command
        .spawn()
        .with_context(|| format!("starting {} daemon", exe.display()))?;
    Ok(())
}

/// A value for a systemd unit line: quoted, with `\`, `"` and `%` (specifiers) escaped, and `$`
/// escaped where the line expands variables (ExecStart). Units are line based, so no newlines.
fn quote(value: &str, exec: bool) -> Result<String> {
    if value.contains(['\n', '\r']) {
        bail!("cannot write a systemd unit with a line break in {value:?}");
    }
    let mut escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    if exec {
        escaped = escaped.replace('$', "$$");
    }
    Ok(format!("\"{escaped}\""))
}

/// A path for `StandardOutput=append:`, which takes it unquoted and expands specifiers.
fn log_path(path: &Path) -> Result<String> {
    let path = path.to_string_lossy();
    if path.contains(['\n', '\r']) {
        bail!("cannot write a systemd unit with a line break in {path:?}");
    }
    Ok(path.replace('%', "%%"))
}

pub(crate) fn service_unit(env: &Env, exe: &Path, log: &Path) -> Result<String> {
    let mut environment = String::new();
    for name in PASSED_VARS {
        if let Some(value) = env.vars.get(name).filter(|value| !value.is_empty()) {
            environment.push_str(&format!(
                "Environment={}\n",
                quote(&format!("{name}={value}"), false)?
            ));
        }
    }
    let log = log_path(log)?;
    Ok(format!(
        "[Unit]\nDescription=Clash of Meat activity collector\n\n\
         [Service]\nExecStart={} daemon\nRestart=always\nRestartSec=5\n{environment}\
         StandardOutput=append:{log}\nStandardError=append:{log}\n\n\
         [Install]\nWantedBy=default.target\n",
        quote(&exe.to_string_lossy(), true)?
    ))
}

fn update_unit(exe: &Path) -> Result<String> {
    Ok(format!(
        "[Unit]\nDescription=Clash of Meat update check\n\n\
         [Service]\nType=oneshot\nExecStart={} update --quiet\n",
        quote(&exe.to_string_lossy(), true)?
    ))
}

const TIMER_UNIT: &str = "[Unit]\nDescription=Daily Clash of Meat update check\n\n\
    [Timer]\nOnCalendar=daily\nRandomizedDelaySec=4h\nPersistent=true\n\n\
    [Install]\nWantedBy=timers.target\n";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(vars: &[(&str, &str)]) -> Env {
        Env {
            home: PathBuf::from("/home/me"),
            vars: vars
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<HashMap<_, _>>(),
        }
    }

    #[test]
    fn service_unit_quotes_paths_and_passes_agent_folders() {
        let unit = service_unit(
            &env(&[
                ("PATH", "/usr/bin:/home/me/.local/bin"),
                ("CODEX_HOME", "/home/me/my \"codex\" 100%"),
                ("GEMINI_CLI_HOME", ""),
            ]),
            Path::new("/home/me/a $dir/clashofmeat"),
            Path::new("/home/me/.clashofmeat/logs/daemon.log"),
        )
        .unwrap();
        assert!(unit.contains("ExecStart=\"/home/me/a $$dir/clashofmeat\" daemon\n"));
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/home/me/.local/bin\"\n"));
        assert!(unit.contains("Environment=\"CODEX_HOME=/home/me/my \\\"codex\\\" 100%%\"\n"));
        assert!(!unit.contains("GEMINI_CLI_HOME"));
        assert!(unit.contains("Restart=always\n"));
        assert!(unit.contains("StandardOutput=append:/home/me/.clashofmeat/logs/daemon.log\n"));
        assert!(unit.contains("WantedBy=default.target\n"));
    }

    #[test]
    fn units_refuse_line_breaks() {
        assert!(service_unit(&env(&[]), Path::new("/tmp/a\nb"), Path::new("/tmp/log")).is_err());
        assert!(service_unit(
            &env(&[("CODEX_HOME", "x\n[Service]")]),
            Path::new("/a"),
            Path::new("/l")
        )
        .is_err());
    }

    #[test]
    fn paths_follow_xdg_variables() {
        assert_eq!(
            installed_exe(&env(&[])),
            Path::new("/home/me/.local/share/clashofmeat/bin/clashofmeat")
        );
        assert_eq!(
            installed_exe(&env(&[("XDG_DATA_HOME", "/data")])),
            Path::new("/data/clashofmeat/bin/clashofmeat")
        );
        // A relative XDG value is invalid by the spec and ignored.
        assert_eq!(
            unit_dir(&env(&[("XDG_CONFIG_HOME", "rel")])),
            Path::new("/home/me/.config/systemd/user")
        );
        assert_eq!(
            unit_dir(&env(&[("XDG_CONFIG_HOME", "/cfg")])),
            Path::new("/cfg/systemd/user")
        );
    }
}
