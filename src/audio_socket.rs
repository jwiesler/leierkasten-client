use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tokio::stream::StreamExt;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::token::*;

pub type SocketToken = Token<CancelableToken<CompletableToken<ValueToken<()>>>>;

pub enum AudioMessage {
    NewResource(StreamStartMessage),
    Audio(Vec<u8>),
}

pub struct AudioSocket {
    address: String,
    token: SocketToken,
    updates: Arc<Mutex<State>>,
    output: Sender<AudioMessage>,
}

impl AudioSocket {
    pub fn new(
        address: String,
        token: SocketToken,
        updates: Arc<Mutex<State>>,
        output: Sender<AudioMessage>,
    ) -> Self {
        AudioSocket {
            address,
            token,
            updates,
            output,
        }
    }
}

pub enum State {
    None,
    Connecting,
    Connected,
    Disconnecting,
}

#[derive(Deserialize)]
pub struct StreamStartMessage {
    pub timestamp: u64,
    pub duration: Option<u64>,
    pub name: String,
}

enum HandleMessageResult {
    Ok,
    Exit,
}

impl AudioSocket {
    async fn handle_message(&mut self, message: Message) -> HandleMessageResult {
        let send_res = match message {
            Message::Text(text) => match serde_json::from_str::<StreamStartMessage>(&text) {
                Ok(message) => self.output.send(AudioMessage::NewResource(message)).await,
                Err(err) => {
                    warn!("Invalid message, failed to parse: {}", err);
                    return HandleMessageResult::Ok;
                }
            },
            Message::Binary(data) => self.output.send(AudioMessage::Audio(data)).await,
            Message::Close(_) => return HandleMessageResult::Exit,
            _ => return HandleMessageResult::Ok,
        };

        match send_res {
            Ok(_) => HandleMessageResult::Ok,
            Err(_) => {
                warn!("AudioMessage receiver disconnected");
                return HandleMessageResult::Exit;
            }
        }
    }

    pub async fn run(mut self) {
        let token = TokenCompleter::new(self.token.clone());
        *self.updates.lock().unwrap() = State::Connecting;

        let (mut stream, _) = connect_async(&self.address)
            .await
            .expect("Failed to connect");
        *self.updates.lock().unwrap() = State::Connected;
        while !token.token().is_canceled() {
            match tokio::time::timeout(Duration::from_millis(20), stream.next()).await {
                Ok(msg) => match msg {
                    Some(msg) => match msg {
                        Ok(msg) => match self.handle_message(msg).await {
                            HandleMessageResult::Ok => (),
                            HandleMessageResult::Exit => break,
                        },
                        // Stream error
                        Err(e) => {
                            info!("{:?}", e);
                            break;
                        }
                    },
                    // End of stream
                    None => break,
                },
                // Timeout
                Err(_) => (),
            }
        }

        *self.updates.lock().unwrap() = State::Disconnecting;
        stream
            .close(None)
            .await
            .expect("Failed to close connection");
    }
}
