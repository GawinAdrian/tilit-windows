//! Verbindet Hotkey, Aufnahme, Spracherkennung und Einfügen (Gegenstück zu `DictationController.swift`).

mod dictation;
mod live_run;
#[cfg(test)]
mod tests;

use crate::live::LiveInput;
use crate::models::SpeechModel;
use crate::store::Store;
use crate::PushToTalkAction;
use async_trait::async_trait;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{sleep_until, Instant};

/// Erst nach dieser Haltezeit startet die Aufnahme: normale Strg-Kürzel lösen nichts aus.
const HOLD_THRESHOLD: Duration = Duration::from_millis(150);
/// Nach dem Loslassen noch kurz weiter aufnehmen, damit das letzte Wort nicht abgeschnitten wird.
const TAIL: Duration = Duration::from_millis(250);
const MINIMUM_SECONDS: f64 = 0.25;
const MAXIMUM_DURATION: Duration = Duration::from_secs(300);
const ERROR_VISIBLE: Duration = Duration::from_secs(6);
/// So lange darf Deepgram nach dem Loslassen die letzten Wörter liefern.
const LIVE_FINISH_TIMEOUT: Duration = Duration::from_secs(3);
/// Pause zwischen zwei Einfügungen, damit die Ziel-App die Zwischenablage liest.
const PASTE_PAUSE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Recording,
    Transcribing,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Start,
    Stop,
    Error,
}

/// Fenster, das den Fokus hat.
pub type FocusTarget = Option<isize>;

/// Mikrofon. Alle Aufrufe kommen aus der Sitzung, nie parallel.
#[async_trait]
pub trait Recorder: Send + Sync {
    /// Startet die Aufnahme; `chunks` bekommt laufend 16-kHz-Audio für die Live-Erkennung.
    async fn start(&self, chunks: Option<UnboundedSender<LiveInput>>) -> Result<(), String>;
    /// Beendet die Aufnahme und gibt alle Samples zurück.
    async fn stop(&self) -> Vec<i16>;
}

/// Einfügen in die aktive App.
pub trait Inserter: Send + Sync {
    fn focus(&self) -> FocusTarget;
    /// Die Zwischenablage wird nur beim ersten Abschnitt gesichert.
    fn paste_segment(&self, text: &str);
    /// Stellt die Zwischenablage kurz nach dem letzten Abschnitt wieder her.
    fn end_session(&self);
    fn copy(&self, text: &str);
}

pub trait SessionUi: Send + Sync {
    fn status(&self, status: &Status);
    /// Vorläufiger Text während der Aufnahme (leer = nichts anzeigen).
    fn interim(&self, text: &str);
    fn sound(&self, sound: Sound);
    fn history_changed(&self);
}

pub trait KeyStore: Send + Sync {
    fn key(&self, model: SpeechModel) -> Option<String>;
}

/// Alles, was die Sitzung von außen bekommt.
#[derive(Clone)]
pub struct SessionDeps {
    pub recorder: Arc<dyn Recorder>,
    pub inserter: Arc<dyn Inserter>,
    pub speech: Arc<dyn crate::speech::SpeechProvider>,
    pub keys: Arc<dyn KeyStore>,
    pub ui: Arc<dyn SessionUi>,
    pub store: Arc<Mutex<Store>>,
    pub last_transcript: Arc<Mutex<Option<String>>>,
}

type Delivery = (Result<String, String>, FocusTarget);

pub struct Session {
    deps: SessionDeps,
    actions: UnboundedReceiver<PushToTalkAction>,
    done_tx: UnboundedSender<Delivery>,
    done_rx: UnboundedReceiver<Delivery>,
    /// Erkennung läuft parallel, das Einfügen aber in der Reihenfolge der Diktate.
    chain: Option<JoinHandle<()>>,
    in_flight: usize,
    recording: bool,
    awaiting_live: bool,
    start_pending: bool,
    error: Option<String>,
    error_deadline: Option<Instant>,
    last_status: Status,
}

/// Sperrt auch nach einer Panik in einem anderen Thread: die Daten bleiben gültiges JSON-Abbild.
pub fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Session {
    /// `actions` kommt vom Hotkey. Ein weggefallenes Mikrofon meldet die Audio-Schicht dort als `Stop`.
    pub fn new(deps: SessionDeps, actions: UnboundedReceiver<PushToTalkAction>) -> Self {
        let (done_tx, done_rx) = unbounded_channel();
        Self {
            deps,
            actions,
            done_tx,
            done_rx,
            chain: None,
            in_flight: 0,
            recording: false,
            awaiting_live: false,
            start_pending: false,
            error: None,
            error_deadline: None,
            last_status: Status::Idle,
        }
    }

    /// Läuft, bis der Hotkey-Kanal geschlossen wird.
    pub async fn run(mut self) {
        while let Some(action) = self.next_action().await {
            if action != PushToTalkAction::Start {
                continue;
            }
            self.dictate().await;
            // Neues Diktat direkt nach dem Loslassen: startet, sobald das vorige abgeschlossen ist.
            while std::mem::take(&mut self.start_pending) {
                self.dictate().await;
            }
        }
    }

    /// Wartet auf die nächste Hotkey-Aktion und erledigt nebenbei fertige Erkennungen und Fehler-Timeouts.
    async fn next_action(&mut self) -> Option<PushToTalkAction> {
        loop {
            let error_deadline = self.error_deadline;
            tokio::select! {
                action = self.actions.recv() => return action,
                Some((result, focus)) = self.done_rx.recv() => self.deliver(result, focus),
                () = until(error_deadline) => {
                    self.error = None;
                    self.error_deadline = None;
                    self.refresh_status();
                }
            }
        }
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.deps.ui.sound(Sound::Error);
        self.error = Some(message.into());
        self.error_deadline = Some(Instant::now() + ERROR_VISIBLE);
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        let status = if let Some(message) = &self.error {
            Status::Error(message.clone())
        } else if self.awaiting_live || self.in_flight > 0 {
            Status::Transcribing
        } else if self.recording {
            Status::Recording
        } else {
            Status::Idle
        };
        if status != self.last_status {
            self.deps.ui.status(&status);
            self.last_status = status;
        }
    }

    /// Merkt das Diktat als letztes und legt es im Verlauf ab.
    fn remember(&self, text: &str) {
        *locked(&self.deps.last_transcript) = Some(text.to_owned());
        locked(&self.deps.store).add_to_history(text);
        self.deps.ui.history_changed();
    }
}

/// Wartet bis zum Zeitpunkt; ohne Zeitpunkt für immer.
async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
