//! Fügt Text per Zwischenablage und Strg+V ein und stellt die Zwischenablage danach wieder her.

use crate::system::wide;
use std::mem::size_of;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tilit_core::session::{locked, FocusTarget, Inserter};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_LCONTROL, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// Markiert unsere eigenen Tastenereignisse, damit der Hotkey sie ignoriert.
pub const SYNTHETIC_MARKER: usize = 0x5449_4C54;
/// Dem Ziel Zeit zum Einlesen geben, bevor die alte Zwischenablage zurückkommt.
const RESTORE_DELAY: Duration = Duration::from_millis(1_000);

/// Inhalt der Zwischenablage: Format und Rohdaten.
type Snapshot = Vec<(u32, Vec<u8>)>;

/// Hält die Zwischenablage offen und schließt sie wieder. Andere Programme können sie kurz blockieren.
struct OpenBoard;

impl OpenBoard {
    fn acquire() -> Result<Self, String> {
        for _ in 0..20 {
            // SAFETY: Ohne Fensterbesitzer; `Drop` schließt wieder.
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Self);
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("Zwischenablage ist blockiert".into())
    }
}

impl Drop for OpenBoard {
    fn drop(&mut self) {
        // SAFETY: Nur erreichbar, wenn `acquire` die Zwischenablage geöffnet hat.
        if unsafe { CloseClipboard() }.is_err() {
            log::warn!("insertion.clipboard.close_failed");
        }
    }
}

/// GDI-Handles und private Formate sind kein kopierbarer Speicher. Bilder gibt es zusätzlich als CF_DIB.
fn is_memory_format(format: u32) -> bool {
    !matches!(format, 2 | 3 | 9 | 14 | 0x80..=0x3FF)
}

/// Kopiert einen Speicherblock der Zwischenablage. Muss bei geöffneter Zwischenablage laufen.
unsafe fn read_block(handle: HANDLE) -> Option<Vec<u8>> {
    let memory = HGLOBAL(handle.0);
    let size = GlobalSize(memory);
    if size == 0 {
        return None;
    }
    let pointer = GlobalLock(memory) as *const u8;
    if pointer.is_null() {
        return None;
    }
    let data = std::slice::from_raw_parts(pointer, size).to_vec();
    let _ = GlobalUnlock(memory);
    Some(data)
}

/// Legt Daten in ein Format. Nach Erfolg gehört der Speicher der Zwischenablage.
unsafe fn write_block(format: u32, data: &[u8]) -> Result<(), String> {
    let memory = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)).map_err(|error| error.to_string())?;
    let pointer = GlobalLock(memory) as *mut u8;
    if pointer.is_null() {
        let _ = GlobalFree(Some(memory));
        return Err("Kein Speicher für die Zwischenablage".into());
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len());
    let _ = GlobalUnlock(memory);
    if let Err(error) = SetClipboardData(format, Some(HANDLE(memory.0))) {
        let _ = GlobalFree(Some(memory));
        return Err(error.to_string());
    }
    Ok(())
}

fn snapshot() -> Result<Snapshot, String> {
    let _board = OpenBoard::acquire()?;
    let mut result = Vec::new();
    let mut format = 0;
    loop {
        // SAFETY: Die Zwischenablage ist über `_board` geöffnet.
        unsafe {
            format = EnumClipboardFormats(format);
            if format == 0 {
                return Ok(result);
            }
            if !is_memory_format(format) {
                continue;
            }
            if let Some(data) = GetClipboardData(format).ok().and_then(|handle| read_block(handle)) {
                result.push((format, data));
            }
        }
    }
}

fn restore(saved: &Snapshot) -> Result<(), String> {
    let _board = OpenBoard::acquire()?;
    // SAFETY: Die Zwischenablage ist über `_board` geöffnet.
    unsafe {
        EmptyClipboard().map_err(|error| error.to_string())?;
        for (format, data) in saved {
            if let Err(message) = write_block(*format, data) {
                log::warn!("insertion.clipboard.restore_format_failed: {message}");
            }
        }
    }
    Ok(())
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
}

fn register_format(name: &str) -> u32 {
    let name = wide(name);
    // SAFETY: `name` ist nullterminiert und lebt während des Aufrufs.
    unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) }
}

/// Schreibt Text. `transient` hält ihn aus Zwischenablage-Verlauf und Cloud-Sync heraus.
fn write_text(text: &str, transient: bool) -> Result<(), String> {
    let _board = OpenBoard::acquire()?;
    // SAFETY: Die Zwischenablage ist über `_board` geöffnet.
    unsafe {
        EmptyClipboard().map_err(|error| error.to_string())?;
        write_block(u32::from(CF_UNICODETEXT.0), &utf16_bytes(text))?;
        if transient {
            write_block(register_format("ExcludeClipboardContentFromMonitorProcessing"), &[0])?;
            write_block(register_format("CanIncludeInClipboardHistory"), &0u32.to_le_bytes())?;
            write_block(register_format("CanUploadToCloudClipboard"), &0u32.to_le_bytes())?;
        }
    }
    Ok(())
}

pub fn copy_text(text: &str) -> Result<(), String> {
    write_text(text, false)
}

fn key(code: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: code, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: SYNTHETIC_MARKER },
        },
    }
}

fn press_paste() {
    let none = KEYBD_EVENT_FLAGS(0);
    let inputs = [
        key(VK_LCONTROL, none),
        key(VK_V, none),
        key(VK_V, KEYEVENTF_KEYUP),
        key(VK_LCONTROL, KEYEVENTF_KEYUP),
    ];
    // SAFETY: `inputs` ist für die Dauer des Aufrufs gültig.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        log::error!("insertion.paste.send_failed");
    }
}

#[derive(Default)]
struct Pending {
    backup: Option<Snapshot>,
    our_sequence: u32,
    /// Jede neue Einfügung macht eine geplante Wiederherstellung ungültig.
    generation: u64,
}

#[derive(Default)]
pub struct ClipboardInserter {
    pending: Arc<Mutex<Pending>>,
}

impl ClipboardInserter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Inserter for ClipboardInserter {
    fn focus(&self) -> FocusTarget {
        // SAFETY: Reine Abfrage ohne Zeiger.
        let window = unsafe { GetForegroundWindow() };
        (!window.is_invalid()).then_some(window.0 as isize)
    }

    fn paste_segment(&self, text: &str) {
        let mut pending = locked(&self.pending);
        pending.generation += 1;
        if pending.backup.is_none() {
            match snapshot() {
                Ok(saved) => pending.backup = Some(saved),
                Err(message) => log::warn!("insertion.clipboard.snapshot_failed: {message}"),
            }
        }
        if let Err(message) = write_text(text, true) {
            log::error!("insertion.clipboard.write_failed: {message}");
            return;
        }
        // SAFETY: Reine Abfrage ohne Zeiger.
        pending.our_sequence = unsafe { GetClipboardSequenceNumber() };
        press_paste();
    }

    fn end_session(&self) {
        let mut guard = locked(&self.pending);
        if guard.backup.is_none() {
            return;
        }
        guard.generation += 1;
        let generation = guard.generation;
        drop(guard);
        let pending = self.pending.clone();
        thread::spawn(move || {
            thread::sleep(RESTORE_DELAY);
            let mut pending = locked(&pending);
            if pending.generation != generation {
                return;
            }
            let Some(saved) = pending.backup.take() else { return };
            // SAFETY: Reine Abfrage ohne Zeiger.
            // Nur wiederherstellen, wenn in der Zwischenzeit niemand sonst kopiert hat.
            if unsafe { GetClipboardSequenceNumber() } == pending.our_sequence {
                if let Err(message) = restore(&saved) {
                    log::warn!("insertion.clipboard.restore_failed: {message}");
                }
            }
        });
    }

    fn copy(&self, text: &str) {
        if let Err(message) = write_text(text, false) {
            log::error!("insertion.clipboard.write_failed: {message}");
        }
    }
}
