/// Aktion, die der Hotkey-Zustandsautomat an die App meldet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushToTalkAction {
    Start,
    Stop,
    /// Eine andere Taste wurde während des Haltens gedrückt (Tastenkombination, kein Diktat).
    Cancel,
}

/// Reiner Zustandsautomat für "rechte Strg halten = aufnehmen".
#[derive(Debug, Default)]
pub struct PushToTalk {
    held: bool,
    /// Nach einem Abbruch bleibt die Taste physisch unten: Wiederholungen dürfen nichts starten.
    suppressed: bool,
}

impl PushToTalk {
    /// Virtueller Tastencode von VK_RCONTROL.
    pub const RIGHT_CONTROL: u32 = 0xA3;

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Rohes Tastenereignis. Windows wiederholt `down`, solange eine Taste gehalten wird.
    pub fn key_event(&mut self, virtual_key: u32, down: bool) -> Option<PushToTalkAction> {
        if virtual_key != Self::RIGHT_CONTROL {
            return if down { self.other_key_down() } else { None };
        }
        if !down {
            return self.reset();
        }
        if self.held || self.suppressed {
            return None;
        }
        self.held = true;
        Some(PushToTalkAction::Start)
    }

    /// Verlorenes Loslassen nachholen (z. B. wenn Windows das Ereignis nicht zugestellt hat).
    pub fn reset(&mut self) -> Option<PushToTalkAction> {
        self.suppressed = false;
        if !self.held {
            return None;
        }
        self.held = false;
        Some(PushToTalkAction::Stop)
    }

    fn other_key_down(&mut self) -> Option<PushToTalkAction> {
        if !self.held {
            return None;
        }
        self.held = false;
        self.suppressed = true;
        Some(PushToTalkAction::Cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PushToTalkAction::{Cancel, Start, Stop};

    const RIGHT: u32 = PushToTalk::RIGHT_CONTROL;
    const LEFT_CONTROL: u32 = 0xA2;
    const KEY_C: u32 = 0x43;

    #[test]
    fn press_and_release_starts_and_stops() {
        let mut ptt = PushToTalk::default();
        assert_eq!(ptt.key_event(RIGHT, true), Some(Start));
        assert!(ptt.is_held());
        assert_eq!(ptt.key_event(RIGHT, false), Some(Stop));
        assert!(!ptt.is_held());
    }

    #[test]
    fn left_control_alone_is_ignored() {
        let mut ptt = PushToTalk::default();
        assert_eq!(ptt.key_event(LEFT_CONTROL, true), None);
        assert!(!ptt.is_held());
    }

    #[test]
    fn repeated_press_does_not_start_twice() {
        let mut ptt = PushToTalk::default();
        assert_eq!(ptt.key_event(RIGHT, true), Some(Start));
        assert_eq!(ptt.key_event(RIGHT, true), None);
    }

    #[test]
    fn key_down_while_held_cancels_once() {
        let mut ptt = PushToTalk::default();
        ptt.key_event(RIGHT, true);
        assert_eq!(ptt.key_event(KEY_C, true), Some(Cancel));
        assert_eq!(ptt.key_event(KEY_C, true), None);
        assert_eq!(ptt.key_event(RIGHT, false), None);
    }

    #[test]
    fn key_repeat_after_cancel_does_not_restart() {
        let mut ptt = PushToTalk::default();
        ptt.key_event(RIGHT, true);
        ptt.key_event(KEY_C, true);
        assert_eq!(ptt.key_event(RIGHT, true), None);
        assert_eq!(ptt.key_event(RIGHT, false), None);
        assert_eq!(ptt.key_event(RIGHT, true), Some(Start));
    }

    #[test]
    fn reset_after_lost_release_emits_stop_once() {
        let mut ptt = PushToTalk::default();
        assert_eq!(ptt.reset(), None);
        ptt.key_event(RIGHT, true);
        assert_eq!(ptt.reset(), Some(Stop));
        assert_eq!(ptt.reset(), None);
        assert!(!ptt.is_held());
    }

    #[test]
    fn key_down_without_hold_is_ignored() {
        let mut ptt = PushToTalk::default();
        assert_eq!(ptt.key_event(KEY_C, true), None);
    }
}
