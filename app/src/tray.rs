//! Tray-Symbol mit Menü: Gegenstück zum macOS-Menüleisten-Eintrag.

use crate::bridge::{notify_changed, status_text, AppState};
use crate::insertion::copy_text;
use crate::system::{self, OpenTarget};
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Wry};
use tilit_core::session::{locked, Status};
use tilit_core::{DictationLanguage, SpeechModel};

const COPY_LAST: &str = "copy-last";
const SHOW_KEYS: &str = "show-keys";
const MICROPHONE: &str = "microphone";
const QUIT: &str = "quit";

/// Menüeinträge, die sich zur Laufzeit ändern.
struct TrayHandles {
    icon: TrayIcon,
    status: MenuItem<Wry>,
    languages: Vec<(DictationLanguage, CheckMenuItem<Wry>)>,
    models: Vec<(SpeechModel, CheckMenuItem<Wry>)>,
}

fn icon_bytes(status: &Status) -> &'static [u8] {
    match status {
        Status::Idle => include_bytes!("../icons/idle.png"),
        Status::Recording => include_bytes!("../icons/recording.png"),
        Status::Transcribing => include_bytes!("../icons/working.png"),
        Status::Error(_) => include_bytes!("../icons/error.png"),
    }
}

pub fn create(app: &App) -> tauri::Result<()> {
    let settings = {
        let state = app.state::<Arc<AppState>>();
        let store = locked(&state.store);
        store.settings().clone()
    };
    let status = MenuItem::new(app, status_text(&Status::Idle), false, None::<&str>)?;

    let mut languages = Vec::new();
    for language in DictationLanguage::ALL {
        let id = format!("language-{}", language.code());
        let checked = language == settings.language;
        languages.push((language, CheckMenuItem::with_id(app, id, language.title(), true, checked, None::<&str>)?));
    }
    let mut models = Vec::new();
    for model in SpeechModel::ALL {
        let id = format!("model-{}", model.key_name());
        let checked = model == settings.speech_model;
        models.push((model, CheckMenuItem::with_id(app, id, model.title(), true, checked, None::<&str>)?));
    }
    let language_menu = Submenu::with_id(app, "languages", "Sprache", true)?;
    for (_, item) in &languages {
        language_menu.append(item)?;
    }
    let model_menu = Submenu::with_id(app, "models", "Modell", true)?;
    for (_, item) in &models {
        model_menu.append(item)?;
    }

    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &language_menu,
            &model_menu,
            &MenuItem::with_id(app, COPY_LAST, "Letztes Diktat kopieren", true, None::<&str>)?,
            &MenuItem::with_id(app, SHOW_KEYS, "Einstellungen…", true, None::<&str>)?,
            &MenuItem::with_id(app, MICROPHONE, "Mikrofon-Einstellungen öffnen…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "TILIT beenden", true, None::<&str>)?,
        ],
    )?;
    let icon = TrayIconBuilder::new()
        .icon(Image::from_bytes(icon_bytes(&Status::Idle))?)
        .tooltip("TILIT")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayHandles { icon, status, languages, models });
    Ok(())
}

fn on_menu(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    match id {
        COPY_LAST => copy_last(app),
        SHOW_KEYS => crate::show_main(app),
        MICROPHONE => system::open(OpenTarget::MicrophoneSettings),
        QUIT => app.exit(0),
        _ => select(app, id),
    }
}

fn copy_last(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let text = locked(&state.last_transcript).clone();
    if let Some(text) = text {
        if let Err(message) = copy_text(&text) {
            log::warn!("tray.copy.failed: {message}");
        }
    }
}

fn select(app: &AppHandle, id: &str) {
    let language = DictationLanguage::ALL.into_iter().find(|item| id == format!("language-{}", item.code()));
    let model = SpeechModel::ALL.into_iter().find(|item| id == format!("model-{}", item.key_name()));
    let state = app.state::<Arc<AppState>>();
    locked(&state.store).update_settings(|settings| {
        if let Some(language) = language {
            settings.language = language;
        }
        if let Some(model) = model {
            settings.speech_model = model;
        }
    });
    show_settings(app);
    notify_changed(app);
}

/// Setzt die Häkchen passend zu den Einstellungen.
pub fn show_settings(app: &AppHandle) {
    let Some(handles) = app.try_state::<TrayHandles>() else { return };
    let state = app.state::<Arc<AppState>>();
    let (language, model) = {
        let store = locked(&state.store);
        (store.settings().language, store.settings().speech_model)
    };
    let language_checks = handles.languages.iter().map(|(item, check)| check.set_checked(*item == language));
    let model_checks = handles.models.iter().map(|(item, check)| check.set_checked(*item == model));
    if language_checks.chain(model_checks).any(|result| result.is_err()) {
        log::warn!("tray.menu.update_failed");
    }
}

pub fn show_status(app: &AppHandle, status: &Status) {
    let Some(handles) = app.try_state::<TrayHandles>() else { return };
    let text = handles.status.set_text(status_text(status));
    let icon = Image::from_bytes(icon_bytes(status)).and_then(|image| handles.icon.set_icon(Some(image)));
    if text.is_err() || icon.is_err() {
        log::warn!("tray.status.update_failed");
    }
}
