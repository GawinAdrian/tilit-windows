use chrono::{DateTime, Utc};
use fancy_regex::{escape, Regex};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sprachwahl für die Erkennung. `Auto` nutzt Deepgrams Mehrsprachen-Modus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DictationLanguage {
    #[default]
    Auto,
    De,
    En,
    Pl,
}

impl DictationLanguage {
    pub const ALL: [Self; 4] = [Self::Auto, Self::De, Self::En, Self::Pl];

    pub fn code(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::De => "de",
            Self::En => "en",
            Self::Pl => "pl",
        }
    }

    pub fn deepgram_code(self) -> &'static str {
        if self == Self::Auto {
            "multi"
        } else {
            self.code()
        }
    }

    /// ISO-Code für Whisper; `None` = automatisch erkennen.
    pub fn whisper_code(self) -> Option<&'static str> {
        (self != Self::Auto).then(|| self.code())
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Auto => "Automatisch (DE/EN/…)",
            Self::De => "Deutsch",
            Self::En => "English",
            Self::Pl => "Polski",
        }
    }
}

/// Spracherkennungs-Modell, das der Nutzer auswählen kann.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeechModel {
    #[default]
    DeepgramNova3,
    GroqWhisper,
}

impl SpeechModel {
    pub const ALL: [Self; 2] = [Self::DeepgramNova3, Self::GroqWhisper];

    pub fn title(self) -> &'static str {
        match self {
            Self::DeepgramNova3 => "Deepgram Nova-3",
            Self::GroqWhisper => "Groq Whisper Large v3 Turbo",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::DeepgramNova3 => {
                "Live beim Sprechen, mehrsprachig (DE/EN/…), Wörterbuch per Keyterm."
            }
            Self::GroqWhisper => {
                "Sehr schnell, gute Polnisch-Erkennung. Text erscheint erst nach dem Loslassen."
            }
        }
    }

    pub fn supports_live(self) -> bool {
        self == Self::DeepgramNova3
    }

    /// Name des Eintrags im Schlüsselspeicher.
    pub fn key_name(self) -> &'static str {
        match self {
            Self::DeepgramNova3 => "DEEPGRAM_API_KEY",
            Self::GroqWhisper => "GROQ_API_KEY",
        }
    }

    pub fn provider_name(self) -> &'static str {
        match self {
            Self::DeepgramNova3 => "Deepgram",
            Self::GroqWhisper => "Groq",
        }
    }
}

/// Eine Ersetzung: `from` wird im erkannten Text durch `to` ersetzt (ganze Wörter, Groß-/Kleinschreibung egal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub id: Uuid,
    pub from: String,
    pub to: String,
}

impl Replacement {
    pub fn new(from: &str, to: &str) -> Self {
        Self { id: Uuid::new_v4(), from: from.into(), to: to.into() }
    }
}

/// Ein Diktat im Verlauf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub date: DateTime<Utc>,
    pub text: String,
}

/// Vom Nutzer gepflegte Einstellungen. Fehlende Felder bekommen ihren Standardwert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserSettings {
    /// Begriffe, die richtig geschrieben werden sollen (Namen, Fachwörter).
    pub dictionary: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub history_enabled: bool,
    pub language: DictationLanguage,
    pub speech_model: SpeechModel,
    pub live_mode: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            dictionary: Vec::new(),
            replacements: Vec::new(),
            history_enabled: true,
            language: DictationLanguage::Auto,
            speech_model: SpeechModel::DeepgramNova3,
            live_mode: true,
        }
    }
}

/// Wendet Ersetzungen auf erkannten Text an.
pub fn apply_replacements(replacements: &[Replacement], text: &str) -> String {
    let mut result = text.to_owned();
    for replacement in replacements {
        let from = replacement.from.trim();
        if from.is_empty() {
            continue;
        }
        // Ganze Wörter: davor und danach kein Buchstabe und keine Ziffer.
        let pattern = format!(r"(?i)(?<![\p{{L}}\p{{N}}]){}(?![\p{{L}}\p{{N}}])", escape(from));
        match Regex::new(&pattern) {
            Ok(regex) => {
                // Closure statt Ersetzungsmuster: `$1` im Zieltext bleibt wörtlich.
                result = regex.replace_all(&result, |_: &fancy_regex::Captures| replacement.to.clone()).into_owned();
            }
            Err(error) => log::warn!("settings.replacement.compile_failed: {error}"),
        }
    }
    result
}

/// Fügt endgültige Textabschnitte zusammen: ab dem zweiten Abschnitt kommt ein Leerzeichen davor.
#[derive(Debug, Default)]
pub struct SegmentJoiner {
    started: bool,
}

impl SegmentJoiner {
    pub fn append(&mut self, segment: &str) -> Option<String> {
        let text = segment.trim();
        if text.is_empty() {
            return None;
        }
        let piece = if self.started { format!(" {text}") } else { text.to_owned() };
        self.started = true;
        Some(piece)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_case_insensitive() {
        let rules = [Replacement::new("slash commit", "/commit")];
        assert_eq!(apply_replacements(&rules, "Dann Slash Commit bitte."), "Dann /commit bitte.");
    }

    #[test]
    fn does_not_replace_inside_words() {
        let rules = [Replacement::new("ki", "KI")];
        assert_eq!(apply_replacements(&rules, "Kind und ki."), "Kind und KI.");
    }

    #[test]
    fn special_characters_are_literal() {
        let rules = [Replacement::new("c++", "C$1++"), Replacement::new("", "x")];
        assert_eq!(apply_replacements(&rules, "ich mag c++ sehr"), "ich mag C$1++ sehr");
    }

    #[test]
    fn settings_decode_old_file_with_missing_keys() -> Result<(), serde_json::Error> {
        assert_eq!(serde_json::from_str::<UserSettings>("{}")?, UserSettings::default());
        Ok(())
    }

    #[test]
    fn joiner_adds_space_from_second_segment_on() {
        let mut joiner = SegmentJoiner::default();
        assert_eq!(joiner.append("Hallo Welt.").as_deref(), Some("Hallo Welt."));
        assert_eq!(joiner.append("Wie geht's?").as_deref(), Some(" Wie geht's?"));
        assert_eq!(joiner.append("   "), None);
        assert_eq!(joiner.append("Gut.").as_deref(), Some(" Gut."));
    }

    #[test]
    fn language_codes() {
        assert_eq!(DictationLanguage::Auto.deepgram_code(), "multi");
        assert_eq!(DictationLanguage::De.deepgram_code(), "de");
        assert_eq!(DictationLanguage::Auto.whisper_code(), None);
    }

    #[test]
    fn only_deepgram_supports_live() {
        assert!(SpeechModel::DeepgramNova3.supports_live());
        assert!(!SpeechModel::GroqWhisper.supports_live());
    }
}
