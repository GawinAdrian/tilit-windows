#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod capture;
mod commands;
mod hotkey;
mod hud;
mod insertion;
mod secrets;
mod sound;
mod system;
mod tray;

use bridge::{AppState, BridgeUi};
use capture::WasapiRecorder;
use insertion::ClipboardInserter;
use secrets::CredentialKeys;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{App, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tilit_core::session::{Session, SessionDeps};
use tilit_core::speech::CloudSpeech;
use tilit_core::store::Store;
use tokio::sync::mpsc::unbounded_channel;

/// Ordner für Einstellungen und Verlauf: %APPDATA%\TILIT.
fn data_directory(app: &App) -> tauri::Result<std::path::PathBuf> {
    Ok(app.path().config_dir()?.join("TILIT"))
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let shown = window.show().and_then(|()| window.unminimize()).and_then(|()| window.set_focus());
        if let Err(error) = shown {
            log::warn!("app.window.show_failed: {error}");
        }
    }
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(Mutex::new(Store::open(&data_directory(app)?)?));
    let (actions, actions_rx) = unbounded_channel();
    let state = Arc::new(AppState::new(store.clone()));
    app.manage(state.clone());

    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("TILIT")
        .inner_size(860.0, 600.0)
        .min_inner_size(720.0, 480.0)
        .center()
        .build()?;
    hud::create(app)?;
    tray::create(app)?;

    let deps = SessionDeps {
        recorder: Arc::new(WasapiRecorder::new(actions.clone())),
        inserter: Arc::new(ClipboardInserter::new()),
        speech: Arc::new(CloudSpeech::default()),
        keys: Arc::new(CredentialKeys),
        ui: Arc::new(BridgeUi::new(app.handle().clone())),
        store,
        last_transcript: state.last_transcript.clone(),
    };
    tauri::async_runtime::spawn(Session::new(deps, actions_rx).run());

    match hotkey::install(actions) {
        Ok(()) => state.hotkey_ok.store(true, Ordering::Relaxed),
        Err(message) => log::error!("hotkey.hook.install_failed: {message}"),
    }
    Ok(())
}

fn main() {
    let result = tauri::Builder::default()
        // Zwei Instanzen hätten zwei Hotkeys und würden doppelt einfügen.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_main(app)))
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .on_window_event(|window, event| {
            // Schließen versteckt nur: TILIT läuft im Tray weiter.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    log::warn!("app.window.hide_failed: {error}");
                }
            }
        })
        .invoke_handler(commands::handler())
        .setup(setup)
        .run(tauri::generate_context!());
    if let Err(error) = result {
        log::error!("app.run.failed: {error}");
    }
}
