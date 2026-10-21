use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{
    webview::PageLoadEvent, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

/// Window-local rectangles (logical px) that take clicks: the hero, its label, an open card or menu.
/// Everything else in the overlay window lets clicks through to the apps underneath.
#[derive(Default)]
pub struct HitRegions(Mutex<Vec<[f64; 4]>>);

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
    window.set_ignore_cursor_events(true)?;
    app.manage(HitRegions::default());
    follow_cursor(window.clone());
    Ok(window)
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// Mouse-down events counted system-wide since login. Reading it needs no permission and
    /// never sees or delays the click itself.
    fn CGEventSourceCounterForEventType(state: i32, event_type: u32) -> u32;
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
}

/// System-wide left clicks so far (0 where the counter is unavailable).
fn left_clicks() -> u32 {
    #[cfg(target_os = "macos")]
    {
        const COMBINED_SESSION_STATE: i32 = 0;
        const LEFT_MOUSE_DOWN: u32 = 1;
        // SAFETY: a pure query of the window server with constant arguments.
        unsafe { CGEventSourceCounterForEventType(COMBINED_SESSION_STATE, LEFT_MOUSE_DOWN) }
    }
    // Windows keeps no count, so the 50 ms poll counts presses itself: a button now down that
    // was up, or one pressed and released since the last read (the low bit).
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        static CLICKS: AtomicU32 = AtomicU32::new(0);
        static DOWN: AtomicBool = AtomicBool::new(false);
        const VK_LBUTTON: i32 = 1;
        // SAFETY: a pure query of the input state with a constant key code.
        let state = unsafe { GetAsyncKeyState(VK_LBUTTON) } as u16;
        let down = state & 0x8000 != 0;
        let was_down = DOWN.swap(down, Ordering::Relaxed);
        if down && !was_down || !down && state & 1 != 0 {
            CLICKS.fetch_add(1, Ordering::Relaxed);
        }
        CLICKS.load(Ordering::Relaxed)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    0
}

#[derive(Clone, Serialize)]
struct ScreenClick {
    x: f64,
    y: f64,
}

/// Polls the cursor 20 times a second and lets clicks reach the overlay only over a hit region.
/// One cheap position read per tick; the window flag changes only when the answer does. A click
/// elsewhere on screen is reported to the overlay (window-local, logical px) so the hero can move away.
fn follow_cursor(window: WebviewWindow) {
    thread::spawn(move || {
        let mut ignoring = true;
        let mut clicks = left_clicks();
        loop {
            thread::sleep(Duration::from_millis(50));
            let visible = window.is_visible().unwrap_or(false);
            let local = (|| {
                let cursor = window.cursor_position().ok()?;
                let origin = window.outer_position().ok()?;
                let scale = window.scale_factor().ok()?;
                Some((
                    (cursor.x - origin.x as f64) / scale,
                    (cursor.y - origin.y as f64) / scale,
                ))
            })();
            let inside = visible
                && local.is_some_and(|(x, y)| {
                    window.state::<HitRegions>().0.lock().is_ok_and(|regions| {
                        regions.iter().any(|[left, top, width, height]| {
                            x >= *left && x <= left + width && y >= *top && y <= top + height
                        })
                    })
                });
            let counted = left_clicks();
            if counted != clicks {
                clicks = counted;
                if let (true, false, Some((x, y))) = (visible, inside, local) {
                    let _ = window.emit_to("hero", "screen-click", ScreenClick { x, y });
                }
            }
            if inside == ignoring {
                ignoring = !inside;
                if window.set_ignore_cursor_events(ignoring).is_err() {
                    break;
                }
            }
        }
    });
}

#[tauri::command]
pub fn hero_hit(window: WebviewWindow, regions: Vec<[f64; 4]>) -> Result<(), String> {
    if regions.len() > 8 || regions.iter().flatten().any(|v| !v.is_finite()) {
        return Err("invalid hit regions".into());
    }
    *window
        .state::<HitRegions>()
        .0
        .lock()
        .map_err(|e| e.to_string())? = regions;
    Ok(())
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
    // Resizing and moving are two steps; the overlay keeps one size so moves never flash.
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let current = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    if (current.width - width).abs() >= 0.5 || (current.height - height).abs() >= 0.5 {
        window
            .set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
    }
    window
        .set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}
