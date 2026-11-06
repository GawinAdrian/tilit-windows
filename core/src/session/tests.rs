//! Sitzungsablauf gegen Fakes. Zeiten stammen aus `DictationController.swift`.

use super::*;
use crate::live::{LiveEvent, LiveHandle};
use crate::models::{Replacement, SpeechModel};
use crate::speech::{SpeechProvider, SpeechRequest};
use crate::PushToTalkAction::{Cancel, Start, Stop};
use std::collections::VecDeque;
use tokio::time::sleep;

const ONE_SECOND: usize = 16_000;

#[derive(Default)]
struct Fakes {
    started: Mutex<usize>,
    samples: Mutex<usize>,
    focus: Mutex<isize>,
    pasted: Mutex<Vec<String>>,
    copied: Mutex<Vec<String>>,
    ended: Mutex<usize>,
    statuses: Mutex<Vec<Status>>,
    requests: Mutex<usize>,
    replies: Mutex<VecDeque<(u64, Result<String, String>)>>,
    live_events: Mutex<Option<UnboundedSender<LiveEvent>>>,
    live_inputs: Mutex<Option<UnboundedReceiver<LiveInput>>>,
    no_key: Mutex<bool>,
}

#[async_trait]
impl Recorder for Fakes {
    async fn start(&self, _: Option<UnboundedSender<LiveInput>>) -> Result<(), String> {
        *locked(&self.started) += 1;
        Ok(())
    }
    async fn stop(&self) -> Vec<i16> {
        vec![0; *locked(&self.samples)]
    }
}

impl Inserter for Fakes {
    fn focus(&self) -> FocusTarget {
        Some(*locked(&self.focus))
    }
    fn paste_segment(&self, text: &str) {
        locked(&self.pasted).push(text.to_owned());
    }
    fn end_session(&self) {
        *locked(&self.ended) += 1;
    }
    fn copy(&self, text: &str) {
        locked(&self.copied).push(text.to_owned());
    }
}

impl SessionUi for Fakes {
    fn status(&self, status: &Status) {
        locked(&self.statuses).push(status.clone());
    }
    fn interim(&self, _: &str) {}
    fn sound(&self, _: Sound) {}
    fn history_changed(&self) {}
}

impl KeyStore for Fakes {
    fn key(&self, _: SpeechModel) -> Option<String> {
        (!*locked(&self.no_key)).then(|| "K".to_owned())
    }
}

#[async_trait]
impl SpeechProvider for Fakes {
    async fn transcribe(&self, _: Vec<u8>, _: SpeechRequest) -> Result<String, String> {
        *locked(&self.requests) += 1;
        let (delay, reply) = locked(&self.replies).pop_front().unwrap_or((0, Err("leer".into())));
        sleep(Duration::from_millis(delay)).await;
        reply
    }
    fn open_live(&self, _: SpeechRequest) -> LiveHandle {
        let (input, input_rx) = unbounded_channel();
        let (events_tx, events) = unbounded_channel();
        *locked(&self.live_events) = Some(events_tx);
        *locked(&self.live_inputs) = Some(input_rx);
        LiveHandle { input, events, abort: None }
    }
}

struct Harness {
    fakes: Arc<Fakes>,
    actions: UnboundedSender<PushToTalkAction>,
    store: Arc<Mutex<Store>>,
}

impl Harness {
    fn new(live: bool) -> Self {
        let fakes = Arc::new(Fakes::default());
        *locked(&fakes.samples) = ONE_SECOND;
        let directory = std::env::temp_dir().join(format!("tilit-session-{}", uuid::Uuid::new_v4()));
        let Ok(mut store) = Store::open(&directory) else { unreachable!("Temp-Ordner fehlt") };
        store.update_settings(|settings| settings.live_mode = live);
        let store = Arc::new(Mutex::new(store));
        let deps = SessionDeps {
            recorder: fakes.clone(),
            inserter: fakes.clone(),
            speech: fakes.clone(),
            keys: fakes.clone(),
            ui: fakes.clone(),
            store: store.clone(),
            last_transcript: Arc::default(),
        };
        let (actions, actions_rx) = unbounded_channel();
        tokio::spawn(Session::new(deps, actions_rx).run());
        Self { fakes, actions, store }
    }

    fn press(&self, action: PushToTalkAction) {
        let _ = self.actions.send(action);
    }

    fn reply(&self, delay: u64, text: &str) {
        locked(&self.fakes.replies).push_back((delay, Ok(text.to_owned())));
    }

    fn live(&self, event: LiveEvent) {
        if let Some(sender) = locked(&self.fakes.live_events).as_ref() {
            let _ = sender.send(event);
        }
    }

    fn pasted(&self) -> Vec<String> {
        locked(&self.fakes.pasted).clone()
    }

    fn history(&self) -> Vec<String> {
        locked(&self.store).history().iter().map(|entry| entry.text.clone()).collect()
    }

    fn last_status(&self) -> Option<Status> {
        locked(&self.fakes.statuses).last().cloned()
    }

    /// Hält die Taste lange genug für eine Aufnahme.
    async fn hold(&self) {
        self.press(Start);
        wait(200).await;
    }

    /// Loslassen plus Nachlauf.
    async fn release(&self) {
        self.press(Stop);
        wait(300).await;
    }
}

async fn wait(milliseconds: u64) {
    sleep(Duration::from_millis(milliseconds)).await;
}

#[tokio::test(start_paused = true)]
async fn tap_under_hold_threshold_does_nothing() {
    let harness = Harness::new(false);
    harness.press(Start);
    wait(100).await;
    harness.press(Stop);
    wait(500).await;
    assert_eq!(*locked(&harness.fakes.started), 0);
}

#[tokio::test(start_paused = true)]
async fn batch_dictation_applies_replacements_and_restores_clipboard() {
    let harness = Harness::new(false);
    locked(&harness.store).update_settings(|settings| {
        settings.replacements.push(Replacement::new("slash commit", "/commit"));
    });
    harness.reply(0, "Dann slash commit bitte.");
    harness.hold().await;
    harness.release().await;
    assert_eq!(harness.pasted(), vec!["Dann /commit bitte."]);
    assert_eq!(*locked(&harness.fakes.ended), 1);
    assert_eq!(harness.history(), vec!["Dann /commit bitte."]);
    assert_eq!(
        *locked(&harness.fakes.statuses),
        vec![Status::Recording, Status::Transcribing, Status::Idle]
    );
}

#[tokio::test(start_paused = true)]
async fn recording_under_minimum_duration_is_dropped() {
    let harness = Harness::new(false);
    *locked(&harness.fakes.samples) = ONE_SECOND / 5;
    harness.hold().await;
    harness.release().await;
    assert_eq!(*locked(&harness.fakes.requests), 0);
    assert_eq!(harness.last_status(), Some(Status::Idle));
}

#[tokio::test(start_paused = true)]
async fn missing_key_shows_error_for_six_seconds() {
    let harness = Harness::new(false);
    *locked(&harness.fakes.no_key) = true;
    harness.hold().await;
    harness.release().await;
    assert!(matches!(harness.last_status(), Some(Status::Error(_))));
    wait(5_000).await;
    assert!(matches!(harness.last_status(), Some(Status::Error(_))));
    wait(1_500).await;
    assert_eq!(harness.last_status(), Some(Status::Idle));
}

#[tokio::test(start_paused = true)]
async fn focus_change_moves_text_to_clipboard() {
    let harness = Harness::new(false);
    harness.reply(100, "Hallo");
    harness.hold().await;
    harness.press(Stop);
    wait(300).await;
    *locked(&harness.fakes.focus) = 2;
    wait(200).await;
    assert!(harness.pasted().is_empty());
    assert_eq!(*locked(&harness.fakes.copied), vec!["Hallo"]);
}

#[tokio::test(start_paused = true)]
async fn cancel_while_recording_discards() {
    let harness = Harness::new(false);
    harness.hold().await;
    harness.press(Cancel);
    wait(500).await;
    assert_eq!(*locked(&harness.fakes.requests), 0);
    assert_eq!(harness.last_status(), Some(Status::Idle));
}

#[tokio::test(start_paused = true)]
async fn results_are_pasted_in_dictation_order() {
    let harness = Harness::new(false);
    harness.reply(2_000, "erstes");
    harness.reply(0, "zweites");
    harness.hold().await;
    harness.release().await;
    harness.hold().await;
    harness.release().await;
    wait(3_000).await;
    assert_eq!(harness.pasted(), vec!["erstes", "zweites"]);
}

#[tokio::test(start_paused = true)]
async fn recording_stops_itself_after_five_minutes() {
    let harness = Harness::new(false);
    harness.reply(0, "lang");
    harness.hold().await;
    wait(301_000).await;
    assert_eq!(harness.pasted(), vec!["lang"]);
}

#[tokio::test(start_paused = true)]
async fn live_segments_are_pasted_while_speaking_and_joined() {
    let harness = Harness::new(true);
    harness.hold().await;
    harness.live(LiveEvent::Interim("Hal".into()));
    harness.live(LiveEvent::Final("Hallo Welt.".into()));
    wait(50).await;
    assert_eq!(harness.pasted(), vec!["Hallo Welt."]);
    harness.live(LiveEvent::Final("Wie geht's?".into()));
    harness.press(Stop);
    wait(300).await;
    harness.live(LiveEvent::Closed { clean: true });
    wait(300).await;
    assert_eq!(harness.pasted(), vec!["Hallo Welt.", " Wie geht's?"]);
    assert_eq!(harness.history(), vec!["Hallo Welt. Wie geht's?"]);
    assert_eq!(*locked(&harness.fakes.requests), 0);
    assert_eq!(harness.last_status(), Some(Status::Idle));
}

#[tokio::test(start_paused = true)]
async fn live_failure_without_text_falls_back_to_batch() {
    let harness = Harness::new(true);
    harness.reply(0, "aus der Aufnahme");
    harness.hold().await;
    harness.live(LiveEvent::Failed("Deepgram Fehler 401 (Key prüfen)".into()));
    harness.live(LiveEvent::Closed { clean: false });
    wait(50).await;
    harness.release().await;
    wait(100).await;
    assert_eq!(harness.pasted(), vec!["aus der Aufnahme"]);
}

#[tokio::test(start_paused = true)]
async fn live_focus_loss_keeps_the_rest_in_the_clipboard() {
    let harness = Harness::new(true);
    harness.hold().await;
    harness.live(LiveEvent::Final("Eins.".into()));
    wait(200).await;
    *locked(&harness.fakes.focus) = 2;
    harness.live(LiveEvent::Final("Zwei.".into()));
    harness.press(Stop);
    wait(300).await;
    harness.live(LiveEvent::Closed { clean: true });
    wait(300).await;
    assert_eq!(harness.pasted(), vec!["Eins."]);
    assert_eq!(*locked(&harness.fakes.copied), vec!["Zwei."]);
}

#[tokio::test(start_paused = true)]
async fn live_timeout_reports_possibly_missing_end() {
    let harness = Harness::new(true);
    harness.hold().await;
    harness.live(LiveEvent::Final("Text.".into()));
    harness.release().await;
    wait(3_100).await;
    assert_eq!(harness.history(), vec!["Text."]);
    assert!(matches!(harness.last_status(), Some(Status::Error(message)) if message.contains("Ende")));
}

#[tokio::test(start_paused = true)]
async fn new_press_after_release_starts_next_dictation() {
    let harness = Harness::new(false);
    harness.reply(0, "eins");
    harness.reply(0, "zwei");
    harness.hold().await;
    harness.press(Stop);
    wait(100).await;
    harness.press(Start);
    wait(500).await;
    harness.release().await;
    assert_eq!(harness.pasted(), vec!["eins", "zwei"]);
}
