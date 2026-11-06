use super::live_run::{next_live, LiveRun};
use super::{
    locked, FocusTarget, Session, Sound, HOLD_THRESHOLD, MAXIMUM_DURATION, MINIMUM_SECONDS, TAIL,
};
use crate::audio::{encode_wav, SAMPLE_RATE};
use crate::models::apply_replacements;
use crate::speech::SpeechRequest;
use crate::PushToTalkAction::{self, Cancel, Start, Stop};
use tokio::time::{sleep, sleep_until, Instant};

/// Ausgang der Phase, in der die Aufnahme gestartet wird.
enum Started {
    Running,
    /// Taste wurde schon während des Starts losgelassen.
    Released,
    Aborted,
}

pub(super) fn seconds(samples: &[i16]) -> f64 {
    samples.len() as f64 / f64::from(SAMPLE_RATE)
}

impl Session {
    /// Ein Diktat vom Tastendruck bis zum Einfügen.
    pub(super) async fn dictate(&mut self) {
        if !self.wait_for_hold().await {
            return;
        }
        self.error = None;
        self.error_deadline = None;
        let mut live = self.open_live();
        let keep_recording = match self.start_recording(&mut live).await {
            Started::Running => true,
            Started::Released => false,
            Started::Aborted => return,
        };
        if keep_recording && !self.record(&mut live).await {
            return;
        }
        self.finish(live).await;
    }

    async fn wait_for_hold(&mut self) -> bool {
        let deadline = Instant::now() + HOLD_THRESHOLD;
        loop {
            tokio::select! {
                () = sleep_until(deadline) => return true,
                action = self.next_action() => {
                    if action != Some(Start) {
                        return false;
                    }
                }
            }
        }
    }

    async fn start_recording(&mut self, live: &mut Option<LiveRun>) -> Started {
        let recorder = self.deps.recorder.clone();
        let chunks = live.as_ref().map(|run| run.handle.input.clone());
        let mut start = Box::pin(async move { recorder.start(chunks).await });
        let mut released = false;
        let mut cancelled = false;
        let result = loop {
            tokio::select! {
                result = &mut start => break result,
                action = self.next_action() => match action {
                    Some(Stop) => released = true,
                    Some(Cancel) | None => cancelled = true,
                    Some(Start) => {}
                },
                event = next_live(live) => self.on_live(live, event, false),
            }
        };
        if let Err(message) = result {
            log::error!("dictation.recorder.start_failed");
            if let Some(run) = live.take() {
                self.stop_live(run);
            }
            self.fail(message);
            return Started::Aborted;
        }
        self.deps.ui.sound(Sound::Start);
        if cancelled {
            self.discard(live.take()).await;
            return Started::Aborted;
        }
        self.recording = true;
        self.refresh_status();
        if released {
            Started::Released
        } else {
            Started::Running
        }
    }

    /// `false`, wenn die Aufnahme verworfen wurde.
    async fn record(&mut self, live: &mut Option<LiveRun>) -> bool {
        // Sicherheitsnetz, falls ein Loslassen verpasst wurde.
        let deadline = Instant::now() + MAXIMUM_DURATION;
        loop {
            tokio::select! {
                action = self.next_action() => match action {
                    Some(Stop) => return true,
                    Some(Cancel) | None => {
                        self.discard(live.take()).await;
                        return false;
                    }
                    Some(Start) => {}
                },
                () = sleep_until(deadline) => return true,
                event = next_live(live) => self.on_live(live, event, true),
            }
        }
    }

    async fn finish(&mut self, mut live: Option<LiveRun>) {
        let focus = self.deps.inserter.focus();
        let tail = sleep(TAIL);
        tokio::pin!(tail);
        loop {
            tokio::select! {
                () = &mut tail => break,
                action = self.next_action() => {
                    if !self.after_release(action) {
                        self.discard(live.take()).await;
                        return;
                    }
                }
                event = next_live(&mut live) => self.on_live(&mut live, event, false),
            }
        }
        let samples = self.deps.recorder.stop().await;
        match live {
            Some(run) => self.complete_live(run, samples, focus).await,
            None => {
                self.recording = false;
                self.process(samples, focus);
            }
        }
    }

    /// Hotkey nach dem Loslassen. `false` bedeutet: das Diktat verwerfen.
    pub(super) fn after_release(&mut self, action: Option<PushToTalkAction>) -> bool {
        match action {
            Some(Start) => self.start_pending = true,
            Some(Stop) => self.start_pending = false,
            Some(Cancel) | None => return false,
        }
        true
    }

    pub(super) async fn discard(&mut self, live: Option<LiveRun>) {
        self.deps.recorder.stop().await;
        if let Some(run) = live {
            self.stop_live(run);
        }
        self.recording = false;
        self.awaiting_live = false;
        self.start_pending = false;
        self.refresh_status();
    }

    /// Schickt die Aufnahme an den Anbieter. Das Ergebnis kommt über `deliver` zurück.
    pub(super) fn process(&mut self, samples: Vec<i16>, focus: FocusTarget) {
        if seconds(&samples) < MINIMUM_SECONDS {
            // Kurzes Antippen: verwerfen.
            self.refresh_status();
            return;
        }
        let (model, language, terms) = {
            let store = locked(&self.deps.store);
            let settings = store.settings();
            (settings.speech_model, settings.language, settings.dictionary.clone())
        };
        let Some(api_key) = self.deps.keys.key(model) else {
            self.fail(format!("{}-Key fehlt (Bereich API-Keys)", model.provider_name()));
            return;
        };
        self.deps.ui.sound(Sound::Stop);
        self.in_flight += 1;
        self.refresh_status();

        let speech = self.deps.speech.clone();
        let done = self.done_tx.clone();
        let previous = self.chain.take();
        let wav = encode_wav(&samples, SAMPLE_RATE);
        let request = SpeechRequest { model, language, api_key, terms };
        self.chain = Some(tokio::spawn(async move {
            let result = speech.transcribe(wav, request).await;
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let _ = done.send((result, focus));
        }));
    }

    pub(super) fn deliver(&mut self, result: Result<String, String>, focus: FocusTarget) {
        self.in_flight -= 1;
        match result {
            Ok(raw) => {
                let replacements = locked(&self.deps.store).settings().replacements.clone();
                let text = apply_replacements(&replacements, &raw);
                self.remember(&text);
                if self.deps.inserter.focus() == focus {
                    self.deps.inserter.paste_segment(&text);
                    self.deps.inserter.end_session();
                    self.refresh_status();
                } else {
                    // Fokus hat gewechselt: nicht in die falsche App einfügen.
                    self.deps.inserter.copy(&text);
                    self.fail("Fokus gewechselt, Text liegt in der Zwischenablage");
                }
            }
            Err(message) => {
                log::error!("dictation.transcription.request_failed");
                self.fail(message);
            }
        }
    }
}
