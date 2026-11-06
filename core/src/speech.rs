use crate::live::{open_deepgram, LiveHandle};
use crate::models::{DictationLanguage, SpeechModel};
use crate::{deepgram, groq};
use async_trait::async_trait;
use std::time::Duration;
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(60);

/// Eingaben einer Erkennung.
#[derive(Debug, Clone)]
pub struct SpeechRequest {
    pub model: SpeechModel,
    pub language: DictationLanguage,
    pub api_key: String,
    pub terms: Vec<String>,
}

/// Spracherkennung hinter einer Schnittstelle, damit die Sitzung ohne Netzwerk testbar ist.
#[async_trait]
pub trait SpeechProvider: Send + Sync {
    async fn transcribe(&self, wav: Vec<u8>, request: SpeechRequest) -> Result<String, String>;
    fn open_live(&self, request: SpeechRequest) -> LiveHandle;
}

/// Echte Anbieter: Deepgram und Groq.
#[derive(Debug, Clone, Default)]
pub struct CloudSpeech {
    http: reqwest::Client,
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl CloudSpeech {
    async fn deepgram_once(&self, wav: Vec<u8>, request: &SpeechRequest, terms: &[String]) -> Result<Reply, String> {
        let response = self
            .http
            .post(deepgram::batch_url(request.language, terms))
            .header("Authorization", deepgram::authorization(&request.api_key))
            .header("Content-Type", "audio/wav")
            .body(wav)
            .timeout(TIMEOUT)
            .send()
            .await;
        read(response, "Deepgram").await
    }

    async fn deepgram(&self, wav: Vec<u8>, request: &SpeechRequest) -> Result<String, String> {
        let mut reply = self.deepgram_once(wav.clone(), request, &request.terms).await?;
        if reply.status == 400 && !request.terms.is_empty() {
            // Manche Sprachen lehnen Wörterbuch-Begriffe ab: noch einmal ohne.
            reply = self.deepgram_once(wav, request, &[]).await?;
        }
        if !(200..300).contains(&reply.status) {
            let message = deepgram::error_message(&reply.body);
            return Err(format!("Deepgram Fehler {}: {message}", reply.status));
        }
        deepgram::parse_transcript(&reply.body).map_err(|error| error.to_string())
    }

    async fn groq(&self, wav: Vec<u8>, request: &SpeechRequest) -> Result<String, String> {
        let boundary = format!("Boundary-{}", Uuid::new_v4());
        let response = self
            .http
            .post(groq::ENDPOINT)
            .header("Authorization", groq::authorization(&request.api_key))
            .header("Content-Type", groq::content_type(&boundary))
            .body(groq::multipart_body(&wav, request.language, &request.terms, &boundary))
            .timeout(TIMEOUT)
            .send()
            .await;
        let reply = read(response, "Groq").await?;
        if !(200..300).contains(&reply.status) {
            return Err(format!("Groq Fehler {}: {}", reply.status, groq::error_message(&reply.body)));
        }
        groq::parse_transcript(&reply.body).map_err(|error| error.to_string())
    }
}

async fn read(response: reqwest::Result<reqwest::Response>, provider: &str) -> Result<Reply, String> {
    let unreachable = |_| format!("Keine Verbindung zu {provider}");
    let response = response.map_err(unreachable)?;
    let status = response.status().as_u16();
    let body = response.bytes().await.map_err(unreachable)?.to_vec();
    Ok(Reply { status, body })
}

#[async_trait]
impl SpeechProvider for CloudSpeech {
    async fn transcribe(&self, wav: Vec<u8>, request: SpeechRequest) -> Result<String, String> {
        match request.model {
            SpeechModel::DeepgramNova3 => self.deepgram(wav, &request).await,
            SpeechModel::GroqWhisper => self.groq(wav, &request).await,
        }
    }

    fn open_live(&self, request: SpeechRequest) -> LiveHandle {
        open_deepgram(request)
    }
}
