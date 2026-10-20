use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Listener, Manager};

use crate::{daemon, settings};

pub struct TrayState {
    menu: Menu<tauri::Wry>,
    title: MenuItem<tauri::Wry>,
    sync_line: MenuItem<tauri::Wry>,
    hero_line: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    update: MenuItem<tauri::Wry>,
    update_ready: AtomicBool,
    hero_displayed: AtomicBool,
}

pub fn set_update_ready(app: &tauri::AppHandle, ready: bool) {
    let state = app.state::<TrayState>();
    let prior = state.update_ready.swap(ready, Ordering::SeqCst);
    if ready && !prior {
        if let Ok(items) = state.menu.items() {
            let _ = state
                .menu
                .insert(&state.update, items.len().saturating_sub(1));
        }
    }
    if !ready && prior {
        let _ = state.menu.remove(&state.update);
    }
}

pub fn refresh(app: &tauri::AppHandle) {
    let state = app.state::<TrayState>();
    let daemon = daemon::state(app);
    let _ = state.title.set_text("Clash of Meat");
    let line = if !daemon.connected {
        "◌ Clash of Meat isn't running".into()
    } else if let Some(status) = daemon.status.as_ref() {
        if status["blocked"].as_bool() == Some(true)
            || (!status["blocked"].is_null() && status["blocked"] != false)
        {
            "⚠ Update required".into()
        } else if status["reauth"].as_bool() == Some(true) {
            "⚠ Reconnect needed".into()
        } else if status["paired"].as_bool() != Some(true) {
            "◌ Not connected".into()
        } else {
            sync_text(status["last_sync_at"].as_str())
        }
    } else {
        "◌ Not connected".into()
    };
    let _ = state.sync_line.set_text(line);
    let hero = daemon
        .heartbeat
        .as_ref()
        .or_else(|| daemon.snapshot.as_ref().and_then(|s| s.get("hero")));
    let label = hero.and_then(hero_text).unwrap_or_default();
    let displayed = !label.is_empty();
    let prior = state.hero_displayed.swap(displayed, Ordering::SeqCst);
    if displayed && !prior {
        let _ = state.menu.insert(&state.hero_line, 2);
    }
    if !displayed && prior {
        let _ = state.menu.remove(&state.hero_line);
    }
    if displayed {
        let _ = state.hero_line.set_text(label);
    }
    let _ = state.pause.set_text(if settings::current(app).show {
        "Pause hero"
    } else {
        "Resume hero"
    });
}

fn sync_text(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "● Connected · not synced yet".into();
    };
    // ISO 8601 UTC timestamps are parsed without a date/time crate.
    let normalized: String = value.chars().take(19).collect();
    let seconds = std::process::Command::new("date")
        .args(["-j", "-u", "-f", "%Y-%m-%dT%H:%M:%S", &normalized, "+%s"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs());
    match (seconds, now) {
        (Some(then), Some(now)) => format!("● Synced {} min ago", now.saturating_sub(then) / 60),
        _ => "● Synced recently".into(),
    }
}

fn hero_text(hero: &Value) -> Option<String> {
    let character = hero.get("character")?;
    let level = character.get("level")?.as_u64()?;
    let name = hero
        .get("player")
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .or_else(|| character.get("heroId").and_then(Value::as_str))
        .unwrap_or("Hero");
    let xp = character.get("xp")?.as_u64()?;
    let formatted = if xp >= 1_000_000 {
        format!("{:.1}M", xp as f64 / 1_000_000.0)
    } else if xp >= 1_000 {
        format!("{:.1}k", xp as f64 / 1_000.0)
    } else {
        xp.to_string()
    };
    Some(format!("Lv {level} {name} · {formatted} XP"))
}

pub fn create(app: &tauri::AppHandle) -> tauri::Result<()> {
    let title = MenuItem::with_id(app, "title", "Clash of Meat", false, None::<&str>)?;
    let sync_line = MenuItem::with_id(
        app,
        "sync_line",
        "◌ Clash of Meat isn't running",
        false,
        None::<&str>,
    )?;
    let hero_line = MenuItem::with_id(app, "hero_line", "", false, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause hero", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
    let open_settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Update ready · Restart", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Clash of Meat", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &title,
            &sync_line,
            &sep1,
            &pause,
            &sync,
            &open_settings,
            &sep2,
            &quit,
        ],
    )?;
    app.manage(TrayState {
        menu: menu.clone(),
        title,
        sync_line,
        hero_line,
        pause,
        update,
        update_ready: AtomicBool::new(false),
        hero_displayed: AtomicBool::new(false),
    });
    TrayIconBuilder::new()
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray.png"
        ))?)
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "pause" => {
                let mut config = settings::current(app);
                config.show = !config.show;
                if let Err(error) = settings::set(app, config) {
                    eprintln!("hero settings: {error}");
                }
            }
            "sync" => {
                let app = app.clone();
                thread::spawn(move || {
                    let _ = daemon::send("sync_now");
                    daemon::refresh_status(&app);
                });
            }
            "settings" => {
                if let Err(error) = settings::show(app) {
                    eprintln!("settings window: {error}");
                }
            }
            "update" => {
                if app.state::<TrayState>().update_ready.load(Ordering::SeqCst) {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = crate::updater::update_restart(app).await {
                            eprintln!("update install: {error}");
                        }
                    });
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    let handle = app.clone();
    app.listen("daemon", move |_| refresh(&handle));
    let handle = app.clone();
    app.listen("hero-settings", move |_| refresh(&handle));
    let handle = app.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(60));
        refresh(&handle);
    });
    refresh(app);
    Ok(())
}
