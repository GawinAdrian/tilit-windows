//! Kleine schwebende Anzeige am unteren Bildschirmrand, solange diktiert wird.
//! Nimmt keinen Fokus und keine Mausklicks an, damit das Zielfeld aktiv bleibt.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{App, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tilit_core::session::Status;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT,
};

const WIDTH: f64 = 440.0;
const HEIGHT: f64 = 64.0;
const BOTTOM_MARGIN: i32 = 48;
const ERROR_VISIBLE: Duration = Duration::from_secs(4);

/// Jede neue Anzeige macht ein geplantes Ausblenden ungültig.
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize)]
struct HudState<'a> {
    phase: &'a str,
    text: &'a str,
}

fn handle(window: &WebviewWindow) -> Option<HWND> {
    window.hwnd().ok().map(|hwnd| HWND(hwnd.0 as _))
}

pub fn create(app: &App) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, "hud", WebviewUrl::App("hud.html".into()))
        .title("TILIT HUD")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .build()?;
    window.set_ignore_cursor_events(true)?;
    if let Some(hwnd) = handle(&window) {
        // SAFETY: `hwnd` gehört zum eben erzeugten Fenster.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let extra = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | extra as isize);
        }
    }
    Ok(())
}

/// Zeigt das Fenster unten mittig auf dem Bildschirm unter dem Mauszeiger, ohne es zu aktivieren.
fn show(window: &WebviewWindow) {
    let (Some(hwnd), Ok(size)) = (handle(window), window.outer_size()) else { return };
    // SAFETY: `hwnd` ist gültig; die Strukturen leben während der Aufrufe.
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of_info(), ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let area = info.rcWork;
        let width = size.width as i32;
        let height = size.height as i32;
        let left = area.left + (area.right - area.left - width) / 2;
        let top = area.bottom - height - BOTTOM_MARGIN;
        let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
        if SetWindowPos(hwnd, Some(HWND_TOPMOST), left, top, width, height, flags).is_err() {
            log::warn!("hud.window.show_failed");
        }
    }
}

fn size_of_info() -> u32 {
    std::mem::size_of::<MONITORINFO>() as u32
}

fn hide(window: &WebviewWindow) {
    if let Some(hwnd) = handle(window) {
        // SAFETY: `hwnd` ist gültig. Der Rückgabewert meldet nur die vorige Sichtbarkeit.
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
}

fn on_main_thread(app: &AppHandle, work: impl FnOnce(&WebviewWindow) + Send + 'static) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("hud") {
            work(&window);
        }
    });
    if let Err(error) = result {
        log::warn!("hud.window.dispatch_failed: {error}");
    }
}

fn emit(app: &AppHandle, phase: &str, text: &str) {
    if let Err(error) = app.emit_to("hud", "hud", HudState { phase, text }) {
        log::warn!("hud.event.emit_failed: {error}");
    }
}

pub fn show_status(app: &AppHandle, status: &Status) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    match status {
        Status::Idle => {
            on_main_thread(app, hide);
            return;
        }
        Status::Recording => emit(app, "recording", "Nimmt auf…"),
        Status::Transcribing => emit(app, "working", "Transkribiert…"),
        Status::Error(message) => {
            emit(app, "error", message);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(ERROR_VISIBLE).await;
                if GENERATION.load(Ordering::SeqCst) == generation {
                    on_main_thread(&app, hide);
                }
            });
        }
    }
    on_main_thread(app, show);
}

/// Zeigt den vorläufigen Text, solange aufgenommen wird (leer = zurück zu "Nimmt auf…").
pub fn show_interim(app: &AppHandle, text: &str) {
    if let Err(error) = app.emit_to("hud", "hud-interim", text) {
        log::warn!("hud.event.emit_failed: {error}");
    }
}
