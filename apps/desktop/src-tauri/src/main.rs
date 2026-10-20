mod daemon;
mod overlay;
mod settings;
mod tray;
mod updater;

use tauri::{ActivationPolicy, Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        let mut config = settings::current(app);
                        config.show = !config.show;
                        if let Err(error) = settings::set(app, config) {
                            eprintln!("hero settings: {error}");
                        }
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            overlay::hero_geometry,
            overlay::hero_place,
            overlay::hero_hit,
            settings::desktop_state,
            settings::daemon_request,
            settings::set_hero_settings,
            settings::open_pair_link,
            updater::update_check,
            updater::update_restart
        ])
        .setup(|app| {
            app.set_activation_policy(ActivationPolicy::Accessory);
            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH);
            if let Err(error) = app.global_shortcut().register(shortcut) {
                eprintln!("could not register Control+Option+H shortcut: {error}");
            }
            let handle = app.handle();
            handle.manage(daemon::DaemonState::default());
            handle.manage(settings::HeroConfig(std::sync::Mutex::new(settings::load(
                handle,
            ))));
            tray::create(handle)?;
            tray::set_update_ready(handle, false);
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
                    Some("quit") => control.exit(0),
                    _ => {}
                }
            });
            overlay::create(handle)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to run Clash of Meat desktop app")
        .run(|app, event| {
            if let tauri::RunEvent::Reopen { .. } = event {
                let mut config = settings::current(app);
                if !config.show {
                    config.show = true;
                    let _ = settings::set(app, config);
                }
            }
        });
}
