mod daemon;
mod overlay;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{ActivationPolicy, Emitter, Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            overlay::hero_geometry,
            overlay::hero_place
        ])
        .setup(|app| {
            app.set_activation_policy(ActivationPolicy::Accessory);
            let handle = app.handle();
            let pause = MenuItem::with_id(handle, "pause", "Pause hero", true, None::<&str>)?;
            let sync = MenuItem::with_id(handle, "sync", "Sync now", true, None::<&str>)?;
            let quit = MenuItem::with_id(handle, "quit", "Quit Clash of Meat", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(handle)?;
            let menu = Menu::with_items(handle, &[&pause, &sync, &separator, &quit])?;
            let paused = Arc::new(AtomicBool::new(false));
            let overlay_paused = paused.clone();
            let overlay_pause_item = pause.clone();
            handle.listen("overlay-control", move |event| {
                let action = serde_json::from_str::<serde_json::Value>(event.payload())
                    .ok()
                    .and_then(|value| value["action"].as_str().map(str::to_owned));
                if matches!(action.as_deref(), Some("pause" | "quit")) {
                    overlay_paused.store(true, Ordering::SeqCst);
                    let _ = overlay_pause_item.set_text("Resume hero");
                }
            });
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/tray.png"
                ))?)
                .icon_as_template(true)
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "pause" => {
                        let showing = app
                            .get_webview_window("hero")
                            .and_then(|window| window.is_visible().ok())
                            .unwrap_or(false);
                        let next = showing && !paused.load(Ordering::SeqCst);
                        paused.store(next, Ordering::SeqCst);
                        let _ = pause.set_text(if next { "Resume hero" } else { "Pause hero" });
                        let _ = app.emit_to(
                            "hero",
                            "hero-control",
                            serde_json::json!({"paused":next,"show":!next}),
                        );
                        if next {
                            if let Some(window) = app.get_webview_window("hero") {
                                let _ = window.hide();
                            }
                        }
                    }
                    "sync" => {
                        std::thread::spawn(|| {
                            if let Err(error) = daemon::send("sync_now") {
                                eprintln!("sync now: {error}");
                            }
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(handle)?;
            overlay::create(handle)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Clash of Meat desktop app");
}
