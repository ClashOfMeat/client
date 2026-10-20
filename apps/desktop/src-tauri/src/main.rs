mod daemon;
mod overlay;
mod settings;
mod tray;
mod updater;

use tauri::{ActivationPolicy, Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            overlay::hero_geometry,
            overlay::hero_place,
            settings::desktop_state,
            settings::daemon_request,
            settings::set_hero_settings,
            settings::open_pair_link,
            updater::update_check,
            updater::update_restart
        ])
        .setup(|app| {
            app.set_activation_policy(ActivationPolicy::Accessory);
            let handle = app.handle();
            handle.manage(daemon::DaemonState::default());
            handle.manage(settings::HeroConfig(std::sync::Mutex::new(settings::load(
                handle,
            ))));
            tray::create(handle)?;
            tray::set_update_ready(handle, false);
            updater::start(handle);
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
                if matches!(action.as_deref(), Some("pause" | "quit")) {
                    let mut config = settings::current(&control);
                    config.show = false;
                    let _ = settings::set(&control, config);
                }
            });
            overlay::create(handle)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Clash of Meat desktop app");
}
