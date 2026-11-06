use super::dictation::seconds;
use super::{
    locked, FocusTarget, Inserter, Session, Sound, LIVE_FINISH_TIMEOUT, MINIMUM_SECONDS, PASTE_PAUSE,
};
use crate::live::{LiveEvent, LiveHandle, LiveInput};
use crate::models::{apply_replacements, SegmentJoiner};
use crate::speech::SpeechRequest;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{sleep, sleep_until, Instant};

/// Zustand einer laufenden Live-Sitzung.
pub(super) struct LiveRun {
    pub(super) handle: LiveHandle,
    pastes: UnboundedSender<String>,
    /// Liefert am Ende den Text, der wegen eines Fokuswechsels nicht eingefügt wurde.
    paster: JoinHandle<String>,
    joiner: SegmentJoiner,
    text: String,
    error: Option<String>,
    closed: Option<bool>,
}

/// Nächstes Live-Ereignis; ohne Live-Sitzung oder nach dem Ende wartet es für immer.
pub(super) async fn next_live(live: &mut Option<LiveRun>) -> LiveEvent {
    if let Some(run) = live {
        if let Some(event) = run.handle.events.recv().await {
            return event;
        }
    }
    std::future::pending().await
}

/// Fügt Abschnitte strikt nacheinander ein. Nach einem Fokuswechsel wird nur noch gesammelt.
async fn paste_in_order(
    mut pieces: UnboundedReceiver<String>,
    inserter: Arc<dyn Inserter>,
    focus: FocusTarget,
) -> String {
    let mut unsent = String::new();
    while let Some(piece) = pieces.recv().await {
        if !unsent.is_empty() || inserter.focus() != focus {
            unsent.push_str(&piece);
            continue;
        }
        inserter.paste_segment(&piece);
        sleep(PASTE_PAUSE).await;
    }
    unsent
}

impl Session {
    pub(super) fn open_live(&mut self) -> Option<LiveRun> {
        let (model, language, terms, enabled) = {
            let store = locked(&self.deps.store);
            let settings = store.settings();
            let enabled = settings.live_mode && settings.speech_model.supports_live();
            (settings.speech_model, settings.language, settings.dictionary.clone(), enabled)
        };
        if !enabled {
            return None;
        }
        let api_key = self.deps.keys.key(model)?;
        let handle = self.deps.speech.open_live(SpeechRequest { model, language, api_key, terms });
        let (pastes, pieces) = unbounded_channel();
        let focus = self.deps.inserter.focus();
        let paster = tokio::spawn(paste_in_order(pieces, self.deps.inserter.clone(), focus));
        Some(LiveRun {
            handle,
            pastes,
            paster,
            joiner: SegmentJoiner::default(),
            text: String::new(),
            error: None,
            closed: None,
        })
    }

    /// Bricht die Live-Sitzung ohne weitere Ausgabe ab.
    pub(super) fn stop_live(&mut self, run: LiveRun) {
        run.handle.cancel();
        run.paster.abort();
        self.deps.ui.interim("");
        self.deps.inserter.end_session();
    }

    pub(super) fn on_live(&mut self, live: &mut Option<LiveRun>, event: LiveEvent, recording: bool) {
        let Some(run) = live else { return };
        match event {
            LiveEvent::Interim(text) => self.deps.ui.interim(&text),
            LiveEvent::Final(text) => {
                self.deps.ui.interim("");
                let replacements = locked(&self.deps.store).settings().replacements.clone();
                if let Some(piece) = run.joiner.append(&apply_replacements(&replacements, &text)) {
                    run.text.push_str(&piece);
                    let _ = run.pastes.send(piece);
                }
            }
            LiveEvent::Failed(message) => {
                log::error!("dictation.live.connection_failed");
                run.error = Some(message.clone());
                // Sofort melden, nicht erst nach dem Loslassen.
                if recording {
                    self.fail(message);
                }
            }
            LiveEvent::Closed { clean } => run.closed = Some(clean),
        }
    }

    /// Nach dem Loslassen: letzte Wörter abwarten, alles einfügen lassen, aufräumen.
    pub(super) async fn complete_live(&mut self, run: LiveRun, samples: Vec<i16>, focus: FocusTarget) {
        self.deps.ui.sound(Sound::Stop);
        self.awaiting_live = true;
        self.refresh_status();
        let _ = run.handle.input.send(LiveInput::Finish);

        let mut live = Some(run);
        let deadline = Instant::now() + LIVE_FINISH_TIMEOUT;
        while live.as_ref().is_some_and(|run| run.closed.is_none()) {
            tokio::select! {
                () = sleep_until(deadline) => break,
                action = self.next_action() => {
                    if !self.after_release(action) {
                        self.discard_finished(live.take());
                        return;
                    }
                }
                event = next_live(&mut live) => self.on_live(&mut live, event, false),
            }
        }
        let Some(run) = live else { return };
        run.handle.cancel();
        let clean = run.closed == Some(true);
        let LiveRun { pastes, paster, text, error, .. } = run;
        // Restliche Abschnitte einfügen lassen.
        drop(pastes);
        let unsent = paster.await.unwrap_or_default();
        let unsent = unsent.trim();
        let error = error.or_else(|| (!clean).then(|| "Live-Verbindung nicht sauber beendet".to_owned()));

        self.awaiting_live = false;
        self.recording = false;
        self.deps.inserter.end_session();
        self.deps.ui.interim("");
        self.conclude_live(&text, unsent, error, samples, focus);
    }

    fn conclude_live(
        &mut self,
        text: &str,
        unsent: &str,
        error: Option<String>,
        samples: Vec<i16>,
        focus: FocusTarget,
    ) {
        if text.is_empty() {
            if seconds(&samples) < MINIMUM_SECONDS {
                self.refresh_status();
            } else if error.is_some() {
                // Live hat nicht geklappt und es wurde nichts eingefügt: ganze Aufnahme normal erkennen.
                self.process(samples, focus);
            } else {
                self.fail("Nichts erkannt");
            }
            return;
        }
        self.remember(text);
        if !unsent.is_empty() {
            self.deps.inserter.copy(unsent);
            self.fail("Fokus gewechselt, Rest liegt in der Zwischenablage");
        } else if let Some(error) = error {
            self.fail(format!("{error}: Ende des Diktats fehlt evtl."));
        } else {
            self.refresh_status();
        }
    }

    /// Abbruch nach dem Loslassen: Der Rekorder ist hier schon gestoppt.
    fn discard_finished(&mut self, live: Option<LiveRun>) {
        if let Some(run) = live {
            self.stop_live(run);
        }
        self.recording = false;
        self.awaiting_live = false;
        self.start_pending = false;
        self.refresh_status();
    }
}
