//! Verbindet die Sitzung aus `tilit-core` mit Fenster, HUD, Tray und Tönen.

use crate::{hud, sound, tray};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tilit_core::session::{locked, SessionUi, Sound, Status};
use tilit_core::store::Store;

/// Gemeinsamer Zustand für Befehle und Sitzung.
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub last_transcript: Arc<Mutex<Option<String>>>,
    pub status: Mutex<Status>,
    pub hotkey_ok: AtomicBool,
}

impl AppState {
    pub fn new(store: Arc<Mutex<Store>>) -> Self {
        Self {
            store,
            last_transcript: Arc::default(),
            status: Mutex::new(Status::Idle),
            hotkey_ok: AtomicBool::new(false),
        }
    }
}

pub fn status_text(status: &Status) -> String {
    match status {
        Status::Idle => "Bereit: rechte Strg halten und sprechen".into(),
        Status::Recording => "Nimmt auf…".into(),
        Status::Transcribing => "Transkribiert…".into(),
        Status::Error(message) => format!("Fehler: {message}"),
    }
}

pub fn status_phase(status: &Status) -> &'static str {
    match status {
        Status::Idle => "idle",
        Status::Recording => "recording",
        Status::Transcribing => "working",
        Status::Error(_) => "error",
    }
}

/// Meldet dem Hauptfenster, dass es seinen Zustand neu laden soll.
pub fn notify_changed(app: &AppHandle) {
    if let Err(error) = app.emit_to("main", "state-changed", ()) {
        log::warn!("app.event.emit_failed: {error}");
    }
}

pub struct BridgeUi {
    app: AppHandle,
}

impl BridgeUi {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl SessionUi for BridgeUi {
    fn status(&self, status: &Status) {
        if let Some(state) = self.app.try_state::<Arc<AppState>>() {
            *locked(&state.status) = status.clone();
        }
        tray::show_status(&self.app, status);
        hud::show_status(&self.app, status);
        notify_changed(&self.app);
    }

    fn interim(&self, text: &str) {
        hud::show_interim(&self.app, text);
    }

    fn sound(&self, sound: Sound) {
        sound::play(sound);
    }

    fn history_changed(&self) {
        notify_changed(&self.app);
    }
}

pub fn is_hotkey_ready(state: &AppState) -> bool {
    state.hotkey_ok.load(Ordering::Relaxed)
}
