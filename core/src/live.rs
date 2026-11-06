use crate::deepgram::{self, LiveResult};
use crate::speech::SpeechRequest;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::AbortHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Ereignis einer Live-Verbindung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    Interim(String),
    Final(String),
    Failed(String),
    /// Die Verbindung ist zu. `clean` nur, wenn der Server den Stream nach CloseStream beendet hat.
    Closed { clean: bool },
}

/// Nachricht an die Live-Verbindung.
#[derive(Debug)]
pub enum LiveInput {
    Audio(Vec<u8>),
    /// Ende der Aufnahme: der Server liefert noch die letzten Wörter und schließt dann.
    Finish,
}

/// Eine Live-Verbindung für genau ein Diktat: Audio rein, Text-Ereignisse raus.
#[derive(Debug)]
pub struct LiveHandle {
    pub input: UnboundedSender<LiveInput>,
    pub events: UnboundedReceiver<LiveEvent>,
    pub abort: Option<AbortHandle>,
}

impl LiveHandle {
    pub fn cancel(&self) {
        if let Some(abort) = &self.abort {
            abort.abort();
        }
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Öffnet die Verbindung im Hintergrund. Audio, das vor dem Verbindungsaufbau kommt, wartet im Kanal.
pub fn open_deepgram(request: SpeechRequest) -> LiveHandle {
    let (input, input_rx) = unbounded_channel();
    let (events_tx, events) = unbounded_channel();
    let task = tokio::spawn(async move {
        let clean = match connect(&request).await {
            Ok(socket) => pump(socket, input_rx, &events_tx).await,
            Err(message) => {
                let _ = events_tx.send(LiveEvent::Failed(message));
                false
            }
        };
        let _ = events_tx.send(LiveEvent::Closed { clean });
    });
    LiveHandle { input, events, abort: Some(task.abort_handle()) }
}

async fn connect(request: &SpeechRequest) -> Result<Socket, String> {
    let url = deepgram::live_url(request.language, &request.terms);
    let mut client_request = url.into_client_request().map_err(|_| "Ungültige Live-Adresse".to_owned())?;
    let token = HeaderValue::from_str(&deepgram::authorization(&request.api_key))
        .map_err(|_| "Key enthält ungültige Zeichen".to_owned())?;
    client_request.headers_mut().insert("Authorization", token);
    match timeout(CONNECT_TIMEOUT, connect_async(client_request)).await {
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(WsError::Http(response))) => Err(status_message(response.status().as_u16())),
        Ok(Err(_)) | Err(_) => Err("Live-Verbindung abgebrochen".into()),
    }
}

fn status_message(status: u16) -> String {
    let hint = if matches!(status, 401 | 403) { " (Key prüfen)" } else { "" };
    format!("Deepgram Fehler {status}{hint}")
}

async fn pump(
    socket: Socket,
    mut input: UnboundedReceiver<LiveInput>,
    events: &UnboundedSender<LiveEvent>,
) -> bool {
    let (mut sink, mut stream) = socket.split();
    let mut finishing = false;
    loop {
        tokio::select! {
            next = input.recv(), if !finishing => {
                let message = match next {
                    Some(LiveInput::Audio(audio)) => Message::Binary(audio.into()),
                    Some(LiveInput::Finish) | None => {
                        finishing = true;
                        Message::Text(r#"{"type":"CloseStream"}"#.into())
                    }
                };
                if sink.send(message).await.is_err() {
                    return closed_unexpectedly(finishing, events);
                }
            }
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let event = deepgram::parse_live(text.as_str()).map(|result| match result {
                        LiveResult::Interim(value) => LiveEvent::Interim(value),
                        LiveResult::Final(value) => LiveEvent::Final(value),
                    });
                    if let Some(event) = event {
                        let _ = events.send(event);
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    return if finishing { true } else { closed_unexpectedly(false, events) };
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => return closed_unexpectedly(finishing, events),
            }
        }
    }
}

/// Abbruch vor CloseStream ist ein Fehler. Danach fehlen höchstens die letzten Wörter.
fn closed_unexpectedly(finishing: bool, events: &UnboundedSender<LiveEvent>) -> bool {
    if !finishing {
        let _ = events.send(LiveEvent::Failed("Live-Verbindung abgebrochen".into()));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failures_hint_at_the_key() {
        assert_eq!(status_message(401), "Deepgram Fehler 401 (Key prüfen)");
        assert_eq!(status_message(500), "Deepgram Fehler 500");
    }
}
