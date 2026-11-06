use crate::models::SpeechModel;
use serde::Serialize;
use std::time::Duration;

/// Ergebnis der Key-Prüfung beim Anbieter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum KeyCheckResult {
    Valid,
    Invalid,
    /// Netzwerkfehler oder unerwartete Antwort: der Key konnte nicht beurteilt werden.
    Unclear(String),
}

/// Harmlose Leseanfrage: Adresse und Authorization-Header je Anbieter.
pub fn probe(model: SpeechModel, api_key: &str) -> (&'static str, String) {
    match model {
        SpeechModel::DeepgramNova3 => {
            ("https://api.deepgram.com/v1/projects", crate::deepgram::authorization(api_key))
        }
        SpeechModel::GroqWhisper => {
            ("https://api.groq.com/openai/v1/models", crate::groq::authorization(api_key))
        }
    }
}

pub fn interpret(status: u16) -> KeyCheckResult {
    match status {
        200..=299 => KeyCheckResult::Valid,
        401 => KeyCheckResult::Invalid,
        other => KeyCheckResult::Unclear(format!("Antwort vom Anbieter: HTTP {other}")),
    }
}

/// Prüft einen Key. Er wird ausschließlich an den jeweiligen Anbieter gesendet.
pub async fn check(model: SpeechModel, api_key: &str) -> KeyCheckResult {
    let key = api_key.trim();
    if key.is_empty() {
        return KeyCheckResult::Invalid;
    }
    let (url, authorization) = probe(model, key);
    let request = reqwest::Client::new().get(url).header("Authorization", authorization).timeout(Duration::from_secs(10));
    match request.send().await {
        Ok(response) => interpret(response.status().as_u16()),
        Err(_) => KeyCheckResult::Unclear("Keine Verbindung zum Anbieter".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepgram_uses_token_on_projects_endpoint() {
        let (url, authorization) = probe(SpeechModel::DeepgramNova3, "K");
        assert_eq!(url, "https://api.deepgram.com/v1/projects");
        assert_eq!(authorization, "Token K");
    }

    #[test]
    fn groq_uses_bearer_on_models_endpoint() {
        let (url, authorization) = probe(SpeechModel::GroqWhisper, "K");
        assert_eq!(url, "https://api.groq.com/openai/v1/models");
        assert_eq!(authorization, "Bearer K");
    }

    #[test]
    fn status_interpretation() {
        assert_eq!(interpret(200), KeyCheckResult::Valid);
        assert_eq!(interpret(401), KeyCheckResult::Invalid);
        assert!(matches!(interpret(500), KeyCheckResult::Unclear(_)));
        assert!(matches!(interpret(403), KeyCheckResult::Unclear(_)));
    }
}
