mod daemon;
mod overlay;
mod settings;
mod updater;

use tauri::{Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;

fn main() {
    tauri::Builder::default()
        // Launching the app while it runs (Start menu, a second double-click) brings the hero back.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_hero(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            overlay::hero_geometry,
            overlay::hero_place,
            overlay::hero_hit,
            settings::desktop_state,
            settings::daemon_request,
            settings::set_hero_settings,
            settings::open_pair_link,
            settings::open_app,
            updater::update_check,
            updater::update_restart
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            // The app starts the daemon on Windows, so it has to start at sign-in.
            #[cfg(windows)]
            {
                use tauri_plugin_autostart::ManagerExt;
                let launcher = app.autolaunch();
                if !launcher.is_enabled().unwrap_or(false) {
                    if let Err(error) = launcher.enable() {
                        eprintln!("start at sign-in: {error}");
                    }
                }
            }
            let handle = app.handle();
            handle.manage(daemon::DaemonState::default());
            handle.manage(settings::HeroConfig(std::sync::Mutex::new(settings::load(
                handle,
            ))));
            updater::start(handle);
            let mut config = settings::current(handle);
            if !config.show {
                config.show = true;
                settings::set(handle, config).map_err(std::io::Error::other)?;
            }
            let updater_handle = handle.clone();
            handle.listen("daemon", move |event| {
                if let Ok(message) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    if message["kind"] == "status" {
                        updater::on_daemon_status(&updater_handle, &message["data"]);
                    }
                }
            });
            let control = handle.clone();
            handle.listen("overlay-control", move |event| {
                let action = serde_json::from_str::<serde_json::Value>(event.payload())
                    .ok()
                    .and_then(|value| value["action"].as_str().map(str::to_owned));
                match action.as_deref() {
                    Some("pause") => {
                        let mut config = settings::current(&control);
                        config.show = false;
                        let _ = settings::set(&control, config);
                    }
                    Some("settings") => {
                        if let Err(error) = settings::show(&control) {
                            eprintln!("settings window: {error}");
                        }
                    }
                    Some("quit") => control.exit(0),
                    _ => {}
                }
            });
            overlay::create(handle)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to run Clash of Meat desktop app")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_hero(_app);
            }
        });
}

fn show_hero(app: &tauri::AppHandle) {
    let mut config = settings::current(app);
    if !config.show {
        config.show = true;
        let _ = settings::set(app, config);
    }
}
