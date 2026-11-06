//! Kleine Windows-Abfragen: Mikrofon-Datenschutz und feste Adressen öffnen.

use serde::Deserialize;
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn consent(path: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 16];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: Puffer und Größe gehören zusammen und leben während des Aufrufs.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            w!("Value"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result.is_err() {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// `false`, wenn der Mikrofonzugriff in den Windows-Datenschutzeinstellungen gesperrt ist.
pub fn microphone_allowed() -> bool {
    let general = w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone");
    let desktop = w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged");
    [general, desktop].into_iter().all(|path| consent(path).as_deref() != Some("Deny"))
}

/// Das Fenster kann nur diese festen Ziele öffnen, keine beliebigen Adressen.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenTarget {
    MicrophoneSettings,
    DeepgramConsole,
    GroqConsole,
}

pub fn open(target: OpenTarget) {
    let address = match target {
        OpenTarget::MicrophoneSettings => w!("ms-settings:privacy-microphone"),
        OpenTarget::DeepgramConsole => w!("https://console.deepgram.com/signup"),
        OpenTarget::GroqConsole => w!("https://console.groq.com/keys"),
    };
    // SAFETY: Alle Zeiger sind statische, nullterminierte Konstanten.
    let result = unsafe { ShellExecuteW(None, w!("open"), address, None, None, SW_SHOWNORMAL) };
    // Werte bis 32 sind laut Windows-API Fehlercodes.
    if result.0 as isize <= 32 {
        log::warn!("app.shell.open_failed");
    }
}
