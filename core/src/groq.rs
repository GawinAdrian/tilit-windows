use crate::deepgram::capped_terms;
use crate::models::DictationLanguage;
use serde_json::Value;

pub const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
pub const MODEL: &str = "whisper-large-v3-turbo";

/// Baut den Multipart-Körper für die Erkennung eines kompletten Diktats.
pub fn multipart_body(wav: &[u8], language: DictationLanguage, terms: &[String], boundary: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 1_024);
    let mut field = |name: &str, value: &str| {
        let part = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n");
        body.extend_from_slice(part.as_bytes());
    };
    field("model", MODEL);
    field("response_format", "json");
    field("temperature", "0");
    if let Some(code) = language.whisper_code() {
        field("language", code);
    }
    // Wörterbuch als Hinweistext: Whisper schreibt genannte Begriffe dann eher richtig.
    let prompt = capped_terms(terms).join(", ");
    if !prompt.is_empty() {
        field("prompt", &prompt);
    }
    let file = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    );
    body.extend_from_slice(file.as_bytes());
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

pub fn content_type(boundary: &str) -> String {
    format!("multipart/form-data; boundary={boundary}")
}

pub fn authorization(api_key: &str) -> String {
    format!("Bearer {api_key}")
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("Unerwartete Antwort von Groq")]
    Invalid,
    #[error("Nichts erkannt")]
    Empty,
}

pub fn parse_transcript(data: &[u8]) -> Result<String, ParseError> {
    let root: Value = serde_json::from_slice(data).map_err(|_| ParseError::Invalid)?;
    let text = root.get("text").and_then(Value::as_str).ok_or(ParseError::Invalid)?.trim();
    if text.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(text.to_owned())
}

pub fn error_message(data: &[u8]) -> String {
    let root: Value = serde_json::from_slice(data).unwrap_or(Value::Null);
    root.pointer("/error/message")
        .or_else(|| root.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("unbekannt")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DictationLanguage::{Auto, Pl};

    #[test]
    fn body_is_multipart_with_model_language_and_prompt() {
        let terms = ["Claude Code".to_owned(), "Gawinnovate".to_owned()];
        let raw = multipart_body(&[1, 2, 3], Pl, &terms, "B");
        let body = String::from_utf8_lossy(&raw);
        assert!(body.contains("whisper-large-v3-turbo"));
        assert!(body.contains("name=\"language\"\r\n\r\npl"));
        assert!(body.contains("Claude Code, Gawinnovate"));
        assert!(body.ends_with("--B--\r\n"));
    }

    #[test]
    fn headers() {
        assert_eq!(authorization("K"), "Bearer K");
        assert_eq!(content_type("B"), "multipart/form-data; boundary=B");
    }

    #[test]
    fn auto_omits_language_and_empty_prompt() {
        let raw = multipart_body(&[], Auto, &[], "B");
        let body = String::from_utf8_lossy(&raw);
        assert!(!body.contains("name=\"language\""));
        assert!(!body.contains("name=\"prompt\""));
    }

    #[test]
    fn parses_transcript_and_error_message() {
        assert_eq!(parse_transcript(br#"{"text":" Hallo. "}"#).as_deref(), Ok("Hallo."));
        assert_eq!(parse_transcript(br#"{"text":""}"#), Err(ParseError::Empty));
        assert_eq!(error_message(br#"{"error":{"message":"Invalid API Key"}}"#), "Invalid API Key");
    }
}
