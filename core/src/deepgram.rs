use crate::models::DictationLanguage;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::Value;

pub const ENDPOINT: &str = "https://api.deepgram.com/v1/listen";
pub const LIVE_ENDPOINT: &str = "wss://api.deepgram.com/v1/listen";

/// Deepgram erlaubt rund 500 Token; wir bleiben deutlich darunter.
pub const MAX_TERMS: usize = 50;
pub const MAX_CHARACTERS: usize = 1_000;

/// Alles außer den URL-sicheren Zeichen (RFC 3986 "unreserved") wird kodiert.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// Wörterbuch-Begriffe auf das erlaubte Maß gekürzt: leere fallen weg, ab dem Limit ist Schluss.
pub fn capped_terms(terms: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut characters = 0;
    for term in terms {
        let trimmed = term.trim();
        if trimmed.is_empty() {
            continue;
        }
        let length = trimmed.chars().count();
        if result.len() >= MAX_TERMS || characters + length > MAX_CHARACTERS {
            break;
        }
        characters += length;
        result.push(trimmed.to_owned());
    }
    result
}

fn query(pairs: &[(&str, &str)], keyterms: &[String]) -> String {
    let terms = capped_terms(keyterms);
    let all = pairs.iter().copied().chain(terms.iter().map(|term| ("keyterm", term.as_str())));
    all.map(|(name, value)| format!("{name}={}", utf8_percent_encode(value, QUERY_VALUE)))
        .collect::<Vec<_>>()
        .join("&")
}

/// URL für die Erkennung eines kompletten Diktats.
pub fn batch_url(language: DictationLanguage, keyterms: &[String]) -> String {
    let pairs = [
        ("model", "nova-3"),
        ("language", language.deepgram_code()),
        ("smart_format", "true"),
        ("punctuate", "true"),
    ];
    format!("{ENDPOINT}?{}", query(&pairs, keyterms))
}

/// URL für die Live-Erkennung per WebSocket.
pub fn live_url(language: DictationLanguage, keyterms: &[String]) -> String {
    let pairs = [
        ("model", "nova-3"),
        ("language", language.deepgram_code()),
        ("encoding", "linear16"),
        ("sample_rate", "16000"),
        ("channels", "1"),
        ("interim_results", "true"),
        ("smart_format", "true"),
        ("punctuate", "true"),
        ("endpointing", "300"),
    ];
    format!("{LIVE_ENDPOINT}?{}", query(&pairs, keyterms))
}

pub fn authorization(api_key: &str) -> String {
    format!("Token {api_key}")
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("Unerwartete Antwort von Deepgram")]
    Invalid,
    #[error("Nichts erkannt")]
    Empty,
}

pub fn parse_transcript(data: &[u8]) -> Result<String, ParseError> {
    let root: Value = serde_json::from_slice(data).map_err(|_| ParseError::Invalid)?;
    let transcript = root
        .pointer("/results/channels/0/alternatives/0/transcript")
        .and_then(Value::as_str)
        .ok_or(ParseError::Invalid)?
        .trim();
    if transcript.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(transcript.to_owned())
}

/// Fehlermeldung aus dem Deepgram-Fehlerkörper, ohne Audio oder Text zu enthalten.
pub fn error_message(data: &[u8]) -> String {
    let root: Value = serde_json::from_slice(data).unwrap_or(Value::Null);
    root.get("err_msg")
        .or_else(|| root.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("unbekannt")
        .to_owned()
}

/// Ereignis aus dem Deepgram-Live-Stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveResult {
    /// Vorläufiger Text, kann sich noch ändern (nur anzeigen).
    Interim(String),
    /// Endgültiger Text eines Abschnitts (darf eingefügt werden).
    Final(String),
}

/// Wertet eine Textnachricht des Servers aus. Alles außer Ergebnissen mit Text liefert `None`.
pub fn parse_live(data: &str) -> Option<LiveResult> {
    let root: Value = serde_json::from_str(data).ok()?;
    if root.get("type")?.as_str()? != "Results" {
        return None;
    }
    let text = root.pointer("/channel/alternatives/0/transcript")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    let is_final = root.get("is_final").and_then(Value::as_bool).unwrap_or(false);
    Some(if is_final { LiveResult::Final(text.into()) } else { LiveResult::Interim(text.into()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DictationLanguage::{Auto, De, Pl};

    #[test]
    fn batch_url_contains_language_and_model() {
        let url = batch_url(Pl, &[]);
        assert!(url.starts_with("https://api.deepgram.com/v1/listen?"));
        assert!(url.contains("model=nova-3"), "{url}");
        assert!(url.contains("language=pl"));
    }

    #[test]
    fn authorization_uses_token_scheme() {
        assert_eq!(authorization("KEY"), "Token KEY");
    }

    #[test]
    fn live_url_has_streaming_parameters() {
        let url = live_url(Auto, &[]);
        assert!(url.starts_with("wss://api.deepgram.com/v1/listen?"));
        let parts = ["model=nova-3", "language=multi", "encoding=linear16", "sample_rate=16000", "channels=1", "interim_results=true"];
        for part in parts {
            assert!(url.contains(part), "{part}");
        }
    }

    #[test]
    fn urls_contain_encoded_keyterms() {
        assert!(live_url(De, &["Claude Code".into()]).contains("keyterm=Claude%20Code"));
    }

    #[test]
    fn keyterms_are_trimmed_and_capped() {
        let terms = capped_terms(&["Claude Code".into(), " ".into(), "Gawinnovate".into()]);
        assert_eq!(terms, vec!["Claude Code", "Gawinnovate"]);
        let many: Vec<String> = (0..200).map(|index| format!("Begriff{index}")).collect();
        assert!(capped_terms(&many).len() <= MAX_TERMS);
    }

    #[test]
    fn parses_transcript() {
        let json = br#"{"results":{"channels":[{"alternatives":[{"transcript":" Hallo Welt. "}]}]}}"#;
        assert_eq!(parse_transcript(json).as_deref(), Ok("Hallo Welt."));
    }

    #[test]
    fn empty_transcript_is_an_error() {
        let json = br#"{"results":{"channels":[{"alternatives":[{"transcript":""}]}]}}"#;
        assert_eq!(parse_transcript(json), Err(ParseError::Empty));
    }

    #[test]
    fn garbage_is_invalid() {
        assert_eq!(parse_transcript(b"nope"), Err(ParseError::Invalid));
    }

    #[test]
    fn extracts_error_message() {
        let data = br#"{"err_code":"INVALID_AUTH","err_msg":"Invalid credentials."}"#;
        assert_eq!(error_message(data), "Invalid credentials.");
    }

    #[test]
    fn parses_final_and_interim() {
        let final_json = r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":" Hallo Welt. "}]}}"#;
        let interim = r#"{"type":"Results","is_final":false,"channel":{"alternatives":[{"transcript":"Hallo"}]}}"#;
        assert_eq!(parse_live(final_json), Some(LiveResult::Final("Hallo Welt.".into())));
        assert_eq!(parse_live(interim), Some(LiveResult::Interim("Hallo".into())));
    }

    #[test]
    fn live_ignores_empty_and_other_messages() {
        let empty = r#"{"type":"Results","is_final":true,"channel":{"alternatives":[{"transcript":""}]}}"#;
        assert_eq!(parse_live(empty), None);
        assert_eq!(parse_live(r#"{"type":"Metadata","request_id":"x"}"#), None);
        assert_eq!(parse_live("kaputt"), None);
    }
}
