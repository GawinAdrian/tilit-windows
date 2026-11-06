//! Plattformneutrale Logik von TILIT: per `cargo test` headless testbar, ohne Mikrofon und Netzwerk.

pub mod audio;
pub mod deepgram;
pub mod groq;
pub mod key_check;
pub mod live;
pub mod models;
pub mod push_to_talk;
pub mod session;
pub mod speech;
pub mod store;

pub use models::{DictationLanguage, HistoryEntry, Replacement, SpeechModel, UserSettings};
pub use push_to_talk::{PushToTalk, PushToTalkAction};
