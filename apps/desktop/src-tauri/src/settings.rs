use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Clone, Serialize, Deserialize)]
pub struct HeroSettings {
    pub show: bool,
    pub screen: String,
    #[serde(default)]
    pub still: bool,
}

impl Default for HeroSettings {
    fn default() -> Self {
        Self {
            show: true,
            screen: "main".into(),
            still: false,
        }
    }
}

pub struct HeroConfig(pub Mutex<HeroSettings>);

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("hero.json"))
}

pub fn load(app: &tauri::AppHandle) -> HeroSettings {
    config_path(app)
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn current(app: &tauri::AppHandle) -> HeroSettings {
    app.state::<HeroConfig>().0.lock().unwrap().clone()
}

pub fn set(app: &tauri::AppHandle, next: HeroSettings) -> Result<HeroSettings, String> {
    if crate::updater::status(app).blocked {
        return Err("This version must be updated before changing hero settings".into());
    }
    if next.screen != "main" && next.screen != "cursor" && !next.screen.starts_with("display:") {
        return Err("invalid screen".into());
    }
    let path = config_path(app)?;
    fs::create_dir_all(path.parent().ok_or("invalid config path")?).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(&next).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    *app.state::<HeroConfig>().0.lock().unwrap() = next.clone();
    // A hidden hero leaves a Dock icon behind; clicking it reopens the app, which shows the hero
    // again and drops the icon (main.rs, RunEvent::Reopen). On Windows, launching the app again
    // from the Start menu does the same (main.rs, single instance).
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(if next.show {
        tauri::ActivationPolicy::Accessory
    } else {
        tauri::ActivationPolicy::Regular
    });
    let _ = app.emit("hero-settings", &next);
    Ok(next)
}

pub fn show(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    let window =
        WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
            .title("Clash of Meat")
            .inner_size(420.0, 520.0)
            .min_inner_size(420.0, 520.0)
            .resizable(false)
            .visible(true)
            .build()?;
    let close_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = close_window.hide();
        }
    });
    window.set_focus()?;
    Ok(())
}

#[tauri::command]
pub fn desktop_state(app: tauri::AppHandle) -> Value {
    #[cfg(target_os = "macos")]
    let name = Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    #[cfg(not(target_os = "macos"))]
    let name = std::env::var("COMPUTERNAME").ok();
    let device = name
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "This PC" } else { "This Mac" }.into());
    json!({"daemon":crate::daemon::state(&app),"hero":current(&app),"displays":displays(&app),"device":device,"updater":crate::updater::status(&app)})
}

#[tauri::command]
pub fn daemon_request(
    app: tauri::AppHandle,
    method: String,
    params: Value,
) -> Result<Value, String> {
    if !matches!(
        method.as_str(),
        "status"
            | "snapshot"
            | "set_provider_enabled"
            | "sync_now"
            | "pair_start"
            | "wire_provider"
            | "evolve"
            | "level_up"
            | "friends"
            | "send_visit"
            | "visit_done"
    ) {
        return Err("unsupported method".into());
    }
    let result = crate::daemon::send_with_params(&method, params)?;
    if method == "status" {
        crate::daemon::publish_status(&app, result.clone());
    }
    Ok(result)
}

#[tauri::command]
pub fn set_hero_settings(
    app: tauri::AppHandle,
    show: bool,
    screen: String,
    still: Option<bool>,
) -> Result<HeroSettings, String> {
    set(
        &app,
        HeroSettings {
            show,
            screen,
            still: still.unwrap_or_else(|| current(&app).still),
        },
    )
}

/// Opens a page of the paired web app. Only the path comes from the page; the address is the
/// one the daemon was installed with.
#[tauri::command]
pub fn open_app(app: tauri::AppHandle, path: String) -> Result<(), String> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err("invalid path".into());
    }
    let base = crate::daemon::state(&app)
        .status
        .and_then(|status| status["app_url"].as_str().map(str::to_owned))
        .ok_or("Clash of Meat isn't connected yet")?;
    open_pair_link(format!("{}{path}", base.trim_end_matches('/')))
}

#[tauri::command]
pub fn open_pair_link(link: String) -> Result<(), String> {
    if !link.starts_with("https://") && !link.starts_with("http://localhost") {
        return Err("invalid pairing link".into());
    }
    #[cfg(windows)]
    let status = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &link])
        .status()
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let status = Command::new("open")
        .arg(&link)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("could not open browser".into())
    }
}

fn displays(app: &tauri::AppHandle) -> Vec<Value> {
    app.available_monitors().unwrap_or_default().into_iter().enumerate()
        .map(|(index, monitor)| json!({"id":format!("display:{index}"),"name":monitor.name().cloned().unwrap_or_else(|| format!("Display {}",index+1))}))
        .collect()
}
