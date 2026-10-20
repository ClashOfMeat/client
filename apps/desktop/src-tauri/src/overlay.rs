use serde::Serialize;
use tauri::{
    webview::PageLoadEvent, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

#[derive(Serialize)]
struct Area {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Serialize)]
pub struct Geometry {
    work_area: Area,
    scale: f64,
}

pub fn create(app: &tauri::AppHandle) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, "hero", WebviewUrl::App("overlay.html".into()))
        .title("Clash of Meat hero")
        .inner_size(160.0, 160.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focusable(false)
        .resizable(false)
        .visible(false)
        .on_page_load(|window, payload| {
            static STARTED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if payload.event() == PageLoadEvent::Finished
                && !STARTED.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                crate::daemon::start(window.app_handle().clone());
            }
        })
        .build()?;
    window.set_focusable(false)?;
    Ok(window)
}

#[tauri::command]
pub fn hero_geometry(window: WebviewWindow, screen: Option<String>) -> Result<Geometry, String> {
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    let selection = screen.as_deref().unwrap_or("main");
    let monitor = if selection == "cursor" {
        let cursor = window.cursor_position().map_err(|e| e.to_string())?;
        monitors
            .iter()
            .find(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                cursor.x >= position.x as f64
                    && cursor.x < (position.x as f64 + size.width as f64)
                    && cursor.y >= position.y as f64
                    && cursor.y < (position.y as f64 + size.height as f64)
            })
            .cloned()
    } else if let Some(index) = selection
        .strip_prefix("display:")
        .and_then(|s| s.parse::<usize>().ok())
    {
        monitors.get(index).cloned()
    } else {
        None
    }
    .or(window.primary_monitor().map_err(|e| e.to_string())?)
    .ok_or("no monitor available")?;
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let size = monitor.size();
    let (x, y, width, height) = if work.size.width > 0 && work.size.height > 0 {
        (
            work.position.x as f64,
            work.position.y as f64,
            work.size.width as f64,
            work.size.height as f64,
        )
    } else {
        let position = monitor.position();
        (
            position.x as f64,
            position.y as f64,
            size.width as f64,
            (size.height as f64 - 80.0 * scale).max(0.0),
        )
    };
    Ok(Geometry {
        work_area: Area {
            x: x / scale,
            y: y / scale,
            width: width / scale,
            height: height / scale,
        },
        scale,
    })
}

#[tauri::command]
pub fn hero_place(
    window: WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width < 1.0 || height < 1.0 {
        return Err("invalid hero rectangle".into());
    }
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}
