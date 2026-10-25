use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST: Duration = Duration::from_secs(3);
const SIX_HOURS: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(10 * 60);
const JITTER: u64 = 30 * 60;
const PLACEHOLDER: &str = "REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY";

#[derive(Clone, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub phase: &'static str,
    pub available: Option<String>,
    pub security: bool,
    pub blocked: bool,
    pub error: Option<String>,
}

pub struct State {
    status: Mutex<UpdateStatus>,
    ready: Mutex<Option<(Update, Vec<u8>)>>,
    busy: AtomicBool,
    kicked_daemon: AtomicBool,
    wake: mpsc::Sender<()>,
}

fn public_key() -> Option<String> {
    let configured = serde_json::from_str::<Value>(include_str!("../tauri.conf.json"))
        .ok()?
        .pointer("/plugins/updater/pubkey")?
        .as_str()?
        .to_owned();
    let key = override_var(
        "CLASHOFMEAT_UPDATE_PUBKEY",
        option_env!("CLASHOFMEAT_UPDATE_PUBKEY"),
    )
    .unwrap_or(configured);
    (!key.trim().is_empty() && key != PLACEHOLDER).then_some(key)
}

/// A test override of the signing key or endpoint: set when the app is built (the end-to-end
/// script does this), or at run time in debug builds only. A release build never takes them from
/// its environment, where any process of the user could plant them (`launchctl setenv`).
fn override_var(name: &str, built: Option<&'static str>) -> Option<String> {
    let runtime = if cfg!(debug_assertions) {
        std::env::var(name).ok()
    } else {
        None
    };
    runtime.or_else(|| built.map(str::to_owned))
}

fn publish(app: &AppHandle) {
    let status = app.state::<State>().status.lock().unwrap().clone();
    if status.blocked {
        let _ = app.emit("hero-control", serde_json::json!({"paused":true}));
        if let Some(window) = app.get_webview_window("hero") {
            let _ = window.hide();
        }
        if let Err(error) = crate::settings::show(app) {
            eprintln!("update settings window: {error}");
        }
    }
    let _ = app.emit("updater", status);
}

fn edit(app: &AppHandle, change: impl FnOnce(&mut UpdateStatus)) {
    change(&mut app.state::<State>().status.lock().unwrap());
    publish(app);
}

pub fn status(app: &AppHandle) -> UpdateStatus {
    app.state::<State>().status.lock().unwrap().clone()
}

fn jitter() -> Duration {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    Duration::from_secs(seed.wrapping_mul(6364136223846793005) % JITTER)
}

pub fn start(app: &AppHandle) {
    let (wake, receiver) = mpsc::channel();
    app.manage(State {
        status: Mutex::new(UpdateStatus {
            current: app.package_info().version.to_string(),
            phase: "up to date",
            available: None,
            security: false,
            blocked: false,
            error: None,
        }),
        ready: Mutex::new(None),
        busy: AtomicBool::new(false),
        kicked_daemon: AtomicBool::new(false),
        wake,
    });
    if public_key().is_none() {
        eprintln!("updater disabled: public key is not configured");
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut wait = FIRST;
        loop {
            if receiver
                .recv_timeout(wait)
                .is_err_and(|error| error == mpsc::RecvTimeoutError::Disconnected)
            {
                break;
            }
            let app_for_check = app.clone();
            tauri::async_runtime::block_on(check(&app_for_check));
            wait = if status(&app).phase == "failed" {
                RETRY
            } else {
                SIX_HOURS + jitter()
            };
        }
    });
}

pub fn on_daemon_status(app: &AppHandle, value: &Value) {
    if value.get("connected").is_some() {
        return;
    }
    let blocked = value
        .get("blocked")
        .is_some_and(|v| !v.is_null() && v != false);
    if blocked && !status(app).blocked {
        edit(app, |status| {
            status.blocked = true;
            if public_key().is_none() {
                status.phase = "failed";
                status.error = Some("Update signing key is not configured".into());
            }
        });
        let _ = app.state::<State>().wake.send(());
    }
    if value
        .get("version")
        .and_then(Value::as_str)
        .is_some_and(|version| version != app.package_info().version.to_string())
    {
        let state = app.state::<State>();
        if !state.kicked_daemon.swap(true, Ordering::SeqCst) {
            std::thread::spawn(restart_daemon);
        }
    }
}

fn restart_daemon() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    if !home
        .join("Library/LaunchAgents/com.clashofmeat.daemon.plist")
        .is_file()
    {
        return;
    }
    let uid = Command::new("id").arg("-u").output();
    let Ok(uid) = uid else {
        eprintln!("updater: could not read uid");
        return;
    };
    if !uid.status.success() {
        eprintln!("updater: could not read uid");
        return;
    }
    let uid = String::from_utf8_lossy(&uid.stdout);
    let job = format!("gui/{}/com.clashofmeat.daemon", uid.trim());
    match Command::new("launchctl")
        .args(["kickstart", "-k", &job])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("updater: daemon restart exited with {status}"),
        Err(error) => eprintln!("updater: daemon restart: {error}"),
    }
}

pub async fn check(app: &AppHandle) {
    let state = app.state::<State>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = fetch(app).await;
    state.busy.store(false, Ordering::SeqCst);
    if let Err(error) = result {
        eprintln!("updater: {error}");
        edit(app, |status| {
            status.phase = "failed";
            status.error = Some(error);
        });
    }
}

async fn fetch(app: &AppHandle) -> Result<(), String> {
    let Some(key) = public_key() else {
        return Err("Update signing key is not configured".into());
    };
    let daemon_config = crate::daemon::send("updater_config").ok();
    // The installation token goes only to the paired app's own update endpoint, never to an
    // overridden one.
    let (endpoint, authorization) = match override_var(
        "CLASHOFMEAT_UPDATE_URL",
        option_env!("CLASHOFMEAT_UPDATE_URL"),
    ) {
        Some(endpoint) => (endpoint, None),
        None => {
            let config = daemon_config
                .as_ref()
                .ok_or("Update endpoint is unavailable")?;
            let endpoint = config["endpoint"]
                .as_str()
                .ok_or("Update endpoint is unavailable")?;
            (
                endpoint.to_owned(),
                config["authorization"].as_str().map(str::to_owned),
            )
        }
    };
    let url: tauri::Url = endpoint
        .parse()
        .map_err(|e| format!("invalid update endpoint: {e}"))?;
    let mut builder = app
        .updater_builder()
        .pubkey(key)
        .endpoints(vec![url.clone()])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(120));
    if let Some(authorization) = &authorization {
        builder = builder
            .header("Authorization", authorization)
            .map_err(|e| e.to_string())?;
    }
    edit(app, |status| {
        status.phase = "checking";
        status.error = None;
    });
    let update = builder
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let Some(mut update) = update else {
        if app.state::<State>().ready.lock().unwrap().is_some() {
            edit(app, |status| status.phase = "ready");
        } else {
            edit(app, |status| {
                status.phase = "up to date";
                status.available = None;
                status.security = false;
            });
        }
        return Ok(());
    };
    let minimum = update.raw_json.get("minimum").and_then(Value::as_str);
    let required = minimum
        .and_then(|minimum| minimum.parse().ok())
        .is_some_and(|minimum| app.package_info().version < minimum);
    let security = update.raw_json["security"].as_bool().unwrap_or(false);
    let version = update.version.clone();
    let held = app
        .state::<State>()
        .ready
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|(held, _)| held.version == version);
    edit(app, |status| {
        status.available = Some(version.clone());
        status.security = security;
        status.blocked |= required;
        status.phase = if held { "ready" } else { "downloading" };
    });
    if held {
        return Ok(());
    }
    // The builder's headers also go on the download, and the archive usually lives on
    // github.com: keep the token only for a download from the app's own origin.
    if !same_origin(&update.download_url, &url) {
        update.headers.remove("authorization");
    }
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    update.verify(&bytes).map_err(|e| e.to_string())?;
    *app.state::<State>().ready.lock().unwrap() = Some((update, bytes));
    edit(app, |status| status.phase = "ready");
    Ok(())
}

fn same_origin(a: &tauri::Url, b: &tauri::Url) -> bool {
    a.origin() == b.origin()
}

#[tauri::command]
pub fn update_check(app: AppHandle) {
    if public_key().is_some() {
        let _ = app.state::<State>().wake.send(());
    } else {
        edit(&app, |status| {
            status.phase = "failed";
            status.error = Some("Update signing key is not configured".into());
        });
    }
}

#[tauri::command]
pub async fn update_restart(app: AppHandle) -> Result<(), String> {
    let ready = app
        .state::<State>()
        .ready
        .lock()
        .unwrap()
        .take()
        .ok_or("No update is ready")?;
    edit(&app, |status| {
        status.phase = "installing";
        status.error = None;
    });
    let result = tauri::async_runtime::spawn_blocking(move || {
        ready.0.install(&ready.1).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    match result {
        Ok(()) => app.restart(),
        Err(error) => {
            edit(&app, |status| {
                status.phase = "failed";
                status.error = Some(format!("Could not install update: {error}"));
            });
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::same_origin;

    #[test]
    fn the_token_stays_on_the_app_origin() {
        let endpoint = "https://app.clashofmeat.com/api/clients/desktop/update?target=x"
            .parse()
            .unwrap();
        for (download, keep) in [
            ("https://app.clashofmeat.com/media/app.tar.gz", true),
            (
                "https://github.com/ClashOfMeat/client/releases/download/x/app.tar.gz",
                false,
            ),
            ("https://app.clashofmeat.com.evil.com/app.tar.gz", false),
            ("http://app.clashofmeat.com/app.tar.gz", false),
            ("https://app.clashofmeat.com:8443/app.tar.gz", false),
        ] {
            assert_eq!(
                same_origin(&download.parse().unwrap(), &endpoint),
                keep,
                "{download}"
            );
        }
    }
}
