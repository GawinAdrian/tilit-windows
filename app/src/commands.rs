//! Befehle für die Einstellungs-UI. API-Keys gehen nur hinein, nie zurück ins Fenster.

use crate::bridge::{is_hotkey_ready, notify_changed, status_phase, status_text, AppState};
use crate::insertion::copy_text;
use crate::{secrets, system, tray};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, State, Wry};
use tilit_core::key_check::{self, KeyCheckResult};
use tilit_core::session::locked;
use tilit_core::{DictationLanguage, HistoryEntry, Replacement, SpeechModel, UserSettings};
use uuid::Uuid;

type Shared<'a> = State<'a, Arc<AppState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfo {
    id: SpeechModel,
    title: &'static str,
    detail: &'static str,
    provider: &'static str,
    live: bool,
    has_key: bool,
}

#[derive(Serialize)]
struct LanguageInfo {
    id: DictationLanguage,
    title: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    phase: &'static str,
    status_text: String,
    settings: UserSettings,
    history: Vec<HistoryEntry>,
    models: Vec<ModelInfo>,
    languages: Vec<LanguageInfo>,
    hotkey_ok: bool,
    microphone_ok: bool,
}

#[tauri::command]
fn snapshot(state: Shared<'_>) -> Snapshot {
    let status = locked(&state.status).clone();
    let store = locked(&state.store);
    let models = SpeechModel::ALL.map(|model| ModelInfo {
        id: model,
        title: model.title(),
        detail: model.detail(),
        provider: model.provider_name(),
        live: model.supports_live(),
        has_key: secrets::has(model),
    });
    Snapshot {
        phase: status_phase(&status),
        status_text: status_text(&status),
        settings: store.settings().clone(),
        history: store.history().to_vec(),
        models: models.into(),
        languages: DictationLanguage::ALL.map(|id| LanguageInfo { id, title: id.title() }).into(),
        hotkey_ok: is_hotkey_ready(&state),
        microphone_ok: system::microphone_allowed(),
    }
}

/// Die Teile der Einstellungen, die das Fenster ändern darf. Fehlende Felder bleiben unverändert.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatch {
    dictionary: Option<Vec<String>>,
    replacements: Option<Vec<Replacement>>,
    history_enabled: Option<bool>,
    language: Option<DictationLanguage>,
    speech_model: Option<SpeechModel>,
    live_mode: Option<bool>,
}

#[tauri::command]
fn update_settings(app: AppHandle, state: Shared<'_>, patch: SettingsPatch) {
    locked(&state.store).update_settings(|settings| {
        if let Some(value) = patch.dictionary {
            settings.dictionary = value;
        }
        if let Some(value) = patch.replacements {
            settings.replacements = value;
        }
        if let Some(value) = patch.history_enabled {
            settings.history_enabled = value;
        }
        if let Some(value) = patch.language {
            settings.language = value;
        }
        if let Some(value) = patch.speech_model {
            settings.speech_model = value;
        }
        if let Some(value) = patch.live_mode {
            settings.live_mode = value;
        }
    });
    tray::show_settings(&app);
    notify_changed(&app);
}

#[tauri::command]
fn remove_history(app: AppHandle, state: Shared<'_>, id: Uuid) {
    locked(&state.store).remove_from_history(id);
    notify_changed(&app);
}

#[tauri::command]
fn clear_history(app: AppHandle, state: Shared<'_>) {
    locked(&state.store).clear_history();
    notify_changed(&app);
}

#[tauri::command]
fn copy_to_clipboard(text: String) -> Result<(), String> {
    copy_text(&text)
}

#[tauri::command]
async fn save_key(app: AppHandle, model: SpeechModel, key: String) -> Result<KeyCheckResult, String> {
    let key = key.trim().to_owned();
    secrets::save(model, &key)?;
    notify_changed(&app);
    Ok(key_check::check(model, &key).await)
}

/// Prüft den eingegebenen Key oder, wenn das Feld leer ist, den gespeicherten.
#[tauri::command]
async fn verify_key(model: SpeechModel, key: Option<String>) -> Result<KeyCheckResult, String> {
    let typed = key.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    let key = typed.or_else(|| secrets::read(model)).ok_or("Noch kein Key")?;
    Ok(key_check::check(model, &key).await)
}

#[tauri::command]
fn delete_key(app: AppHandle, model: SpeechModel) -> Result<(), String> {
    secrets::delete(model)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
fn open_target(target: system::OpenTarget) {
    system::open(target);
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke<Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        snapshot,
        update_settings,
        remove_history,
        clear_history,
        copy_to_clipboard,
        save_key,
        verify_key,
        delete_key,
        open_target
    ]
}
