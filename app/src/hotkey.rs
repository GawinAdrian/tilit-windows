//! Globaler Tastenabgriff für Push-to-Talk (`WH_KEYBOARD_LL`). Braucht unter Windows keine Berechtigung.

use crate::insertion::SYNTHETIC_MARKER;
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tilit_core::session::locked;
use tilit_core::{PushToTalk, PushToTalkAction};
use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// Wie oft geprüft wird, ob ein Loslassen verpasst wurde.
const WATCH_INTERVAL: Duration = Duration::from_millis(200);

struct Hook {
    ptt: Mutex<PushToTalk>,
    actions: UnboundedSender<PushToTalkAction>,
}

/// Der Hook-Callback bekommt keinen Kontext übergeben, deshalb liegt der Zustand global.
static HOOK: OnceLock<Hook> = OnceLock::new();

/// Den Callback sofort verlassen: ein langsamer Hook bremst die Tastatur im ganzen System.
unsafe extern "system" fn on_key(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = &*(data.0 as *const KBDLLHOOKSTRUCT);
        let kind = message.0 as u32;
        let down = kind == WM_KEYDOWN || kind == WM_SYSKEYDOWN;
        let up = kind == WM_KEYUP || kind == WM_SYSKEYUP;
        // Eigenes Strg+V darf das Diktieren nicht abbrechen.
        if (down || up) && event.dwExtraInfo != SYNTHETIC_MARKER {
            if let Some(hook) = HOOK.get() {
                if let Some(action) = locked(&hook.ptt).key_event(event.vkCode, down) {
                    let _ = hook.actions.send(action);
                }
            }
        }
    }
    CallNextHookEx(None, code, message, data)
}

fn run_hook(ready: &mpsc::Sender<Result<(), String>>) {
    // SAFETY: Der Hook lebt bis zum Prozessende auf diesem Thread, der die Nachrichtenschleife betreibt.
    unsafe {
        let module = match GetModuleHandleW(None) {
            Ok(module) => module,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        if let Err(error) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), Some(module.into()), 0) {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
        let _ = ready.send(Ok(()));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            DispatchMessageW(&message);
        }
    }
}

/// Holt ein verlorenes Loslassen nach, damit keine Aufnahme hängen bleibt.
fn watch_release() {
    loop {
        thread::sleep(WATCH_INTERVAL);
        let Some(hook) = HOOK.get() else { continue };
        // SAFETY: Reine Abfrage ohne Zeiger.
        let pressed = unsafe { GetAsyncKeyState(PushToTalk::RIGHT_CONTROL as i32) } < 0;
        let mut ptt = locked(&hook.ptt);
        if ptt.is_held() && !pressed {
            if let Some(action) = ptt.reset() {
                let _ = hook.actions.send(action);
            }
        }
    }
}

pub fn install(actions: UnboundedSender<PushToTalkAction>) -> Result<(), String> {
    let hook = Hook { ptt: Mutex::default(), actions };
    HOOK.set(hook).map_err(|_| "Hotkey ist schon installiert".to_owned())?;
    let (ready, result) = mpsc::channel();
    thread::spawn(move || run_hook(&ready));
    thread::spawn(watch_release);
    result.recv().map_err(|error| error.to_string())?
}
