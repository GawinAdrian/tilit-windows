//! API-Keys liegen in der Windows-Anmeldeinformationsverwaltung, ein Eintrag pro Anbieter.

use keyring::{Entry, Error};
use tilit_core::session::KeyStore;
use tilit_core::SpeechModel;

const SERVICE: &str = "de.gawinnovate.tilit";

fn entry(model: SpeechModel) -> Result<Entry, String> {
    Entry::new(SERVICE, model.key_name()).map_err(|_| "Schlüsselspeicher nicht verfügbar".to_owned())
}

pub fn read(model: SpeechModel) -> Option<String> {
    match entry(model).ok()?.get_password() {
        Ok(key) if !key.is_empty() => Some(key),
        Ok(_) | Err(Error::NoEntry) => None,
        Err(_) => {
            log::error!("secrets.credential.read_failed");
            None
        }
    }
}

pub fn has(model: SpeechModel) -> bool {
    read(model).is_some()
}

pub fn save(model: SpeechModel, key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("Key ist leer".into());
    }
    entry(model)?.set_password(key).map_err(|_| "Konnte den Key nicht speichern".to_owned())
}

pub fn delete(model: SpeechModel) -> Result<(), String> {
    match entry(model)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(_) => Err("Konnte den Key nicht entfernen".into()),
    }
}

pub struct CredentialKeys;

impl KeyStore for CredentialKeys {
    fn key(&self, model: SpeechModel) -> Option<String> {
        read(model)
    }
}
