use crate::models::{HistoryEntry, UserSettings};
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const HISTORY_LIMIT: usize = 200;

/// Persistenz für Einstellungen und Verlauf als JSON im Ordner des Nutzers.
#[derive(Debug)]
pub struct Store {
    settings: UserSettings,
    history: Vec<HistoryEntry>,
    settings_path: PathBuf,
    history_path: PathBuf,
}

impl Store {
    pub fn open(directory: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(directory)?;
        let settings_path = directory.join("settings.json");
        let history_path = directory.join("history.json");
        Ok(Self {
            settings: read(&settings_path).unwrap_or_default(),
            history: read(&history_path).unwrap_or_default(),
            settings_path,
            history_path,
        })
    }

    pub fn settings(&self) -> &UserSettings {
        &self.settings
    }

    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    pub fn update_settings(&mut self, change: impl FnOnce(&mut UserSettings)) {
        change(&mut self.settings);
        write(&self.settings, &self.settings_path);
    }

    pub fn add_to_history(&mut self, text: &str) {
        if !self.settings.history_enabled || text.is_empty() {
            return;
        }
        let entry = HistoryEntry { id: Uuid::new_v4(), date: Utc::now(), text: text.to_owned() };
        self.history.insert(0, entry);
        self.history.truncate(HISTORY_LIMIT);
        write(&self.history, &self.history_path);
    }

    pub fn remove_from_history(&mut self, id: Uuid) {
        self.history.retain(|entry| entry.id != id);
        write(&self.history, &self.history_path);
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        write(&self.history, &self.history_path);
    }
}

/// Eine kaputte Datei wird beiseitegelegt, damit der nächste Schreibvorgang sie nicht überschreibt.
fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let data = fs::read(path).ok()?;
    match serde_json::from_slice(&data) {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("store.file.read_failed: line {}", error.line());
            if let Err(error) = fs::rename(path, path.with_extension("corrupt.json")) {
                log::warn!("store.file.quarantine_failed: {error}");
            }
            None
        }
    }
}

/// Atomar: erst in eine Nachbardatei schreiben, dann umbenennen.
fn write<T: Serialize>(value: &T, path: &Path) {
    let temporary = path.with_extension("tmp");
    let result = serde_json::to_vec_pretty(value)
        .map_err(std::io::Error::from)
        .and_then(|data| fs::write(&temporary, data))
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = result {
        log::error!("store.file.write_failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tilit-{name}-{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&path);
        path
    }

    #[test]
    fn settings_and_history_survive_reopen() -> std::io::Result<()> {
        let path = directory("reopen");
        let mut store = Store::open(&path)?;
        store.update_settings(|settings| settings.dictionary.push("TILIT".into()));
        store.add_to_history("Hallo");
        let reopened = Store::open(&path)?;
        assert_eq!(reopened.settings().dictionary, vec!["TILIT"]);
        assert_eq!(reopened.history()[0].text, "Hallo");
        Ok(())
    }

    #[test]
    fn history_is_capped_newest_first() -> std::io::Result<()> {
        let mut store = Store::open(&directory("cap"))?;
        for index in 0..=HISTORY_LIMIT {
            store.add_to_history(&index.to_string());
        }
        assert_eq!(store.history().len(), HISTORY_LIMIT);
        assert_eq!(store.history()[0].text, HISTORY_LIMIT.to_string());
        Ok(())
    }

    #[test]
    fn disabled_history_stores_nothing() -> std::io::Result<()> {
        let mut store = Store::open(&directory("off"))?;
        store.update_settings(|settings| settings.history_enabled = false);
        store.add_to_history("geheim");
        assert!(store.history().is_empty());
        Ok(())
    }

    #[test]
    fn corrupt_file_is_kept_aside() -> std::io::Result<()> {
        let path = directory("corrupt");
        fs::write(path.join("settings.json"), "kaputt")?;
        let store = Store::open(&path)?;
        assert_eq!(store.settings(), &UserSettings::default());
        assert!(path.join("settings.corrupt.json").exists());
        Ok(())
    }
}
