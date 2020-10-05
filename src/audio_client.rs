use std::collections::VecDeque;

use crate::single_buffer_sender::SingleBufferSender;
use crate::{Cancelable, PlayerContext, PlayerToken, PlayingInfo, State, TokenCompleter};
use audiopus::coder::Decoder;
use audiopus::{Channels, SampleRate};
use futures_util::core_reexport::time::Duration;
use serde::Deserialize;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use tokio::stream::StreamExt;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub struct AudioClient {
    decoder: Decoder,
    sender: SingleBufferSender<Vec<f32>>,
    timestamp: u64,
    buffer: VecDeque<Message>,
    buffering: bool,
    context: Arc<PlayerContext>,
}

impl AudioClient {
    pub fn new(sender: Sender<Vec<f32>>, context: Arc<PlayerContext>) -> Self {
        AudioClient {
            decoder: Decoder::new(SampleRate::Hz48000, Channels::Stereo).unwrap(),
            sender: SingleBufferSender::new(sender),
            timestamp: 0,
            buffer: VecDeque::with_capacity(50),
            buffering: true,
            context,
        }
    }
}

pub const SAMPLES_PER_FRAME: u64 = 960;
pub const SAMPLE_RATE: u64 = 48000;

#[derive(Deserialize)]
struct StreamStartMessage {
    timestamp: u64,
    duration: Option<u64>,
    name: String,
}

impl AudioClient {
    fn update_timestamp(&mut self, timestamp: u64) {
        // let old = self.seconds;
        self.timestamp = timestamp;
        self.context.set_timestamp(self.timestamp);
        // if old != self.seconds {
        //     info!("Timestamp: {}s, buffer: {}", self.seconds, self.buffer.len());
        // }
    }

    async fn try_consume(&mut self) {
        if self.buffering {
            self.buffering = self.buffer.len() < 50;
            if !self.buffering {
                let mut state = self.context.state();
                match state.deref_mut() {
                    State::Playing(info) => info.buffering = false,
                    State::Buffering => *state = State::Preparing,
                    _ => panic!(),
                }
                info!("Finished buffering");
            }
        } else {
            self.buffering = self.buffer.is_empty();
            if self.buffering {
                match self.context.state().deref_mut() {
                    State::Playing(info) => info.buffering = true,
                    _ => panic!(),
                }
                info!("Buffering");
            }
        }
        if !self.buffering {
            match self.sender.try_flush() {
                Ok(flushed) => {
                    if !flushed {
                        return;
                    }
                }
                Err(_) => panic!(),
            }
            match self.buffer.pop_front() {
                Some(m) => {
                    self.context.set_buffer(self.buffer.len());
                    self.handle_message(m).await
                }
                None => (),
            }
        }
    }

    async fn handle_message(&mut self, message: tokio_tungstenite::tungstenite::Message) {
        match message {
            Message::Text(text) => {
                info!("Message: {}", &text);
                match serde_json::from_str::<StreamStartMessage>(&text) {
                    Ok(message) => {
                        info!(
                            "Playing \"{}\" buffer {}, buffering {}",
                            message.name,
                            self.buffer.len(),
                            self.buffering
                        );
                        {
                            let mut state = self.context.state();
                            match state.deref() {
                                State::Preparing | State::Playing(_) => {
                                    *state = State::Playing(Box::new(PlayingInfo {
                                        name: message.name,
                                        duration: message.duration,
                                        buffering: self.buffering,
                                    }))
                                }
                                _ => panic!(),
                            };
                        }
                        self.update_timestamp(message.timestamp);
                    }
                    Err(err) => warn!("Invalid message, failed to parse: {}", err),
                }
            }
            Message::Binary(data) => {
                self.update_timestamp(self.timestamp + SAMPLES_PER_FRAME);
                let mut buffer = Vec::with_capacity(512 * 12);
                buffer.resize(512 * 12, 0.0);
                let res = self
                    .decoder
                    .decode_float(Some(data.as_slice()), buffer.as_mut_slice(), false)
                    .unwrap()
                    * 2;
                buffer.resize(res, 0.0);
                match self.sender.send_or_store(buffer) {
                    Ok(_) => (),
                    Err(_) => panic!(),
                }
            }
            _ => (),
        }
    }

    pub async fn run(&mut self, address: String, token: PlayerToken) {
        let token = TokenCompleter::new(token);
        {
            let mut info = self.context.state();
            match info.deref() {
                State::None => *info = State::Connecting,
                _ => panic!(),
            }
        }

        info!("Connecting to {}", &address);
        let (mut stream, _) = connect_async(address).await.unwrap();

        {
            let mut info = self.context.state();
            match info.deref() {
                State::Connecting => *info = State::Buffering,
                _ => panic!(),
            }
        }
        while !token.token().is_canceled() {
            match tokio::time::timeout(Duration::from_millis(20), stream.next()).await {
                Ok(msg) => match msg {
                    Some(msg) => match msg {
                        Ok(msg) => {
                            self.buffer.push_back(msg);
                            self.context.set_buffer(self.buffer.len());
                        }
                        Err(_) => break,
                    },
                    None => break,
                },
                Err(_) => (),
            }
            self.try_consume().await;
        }

        info!("Audio client exiting");
        stream
            .close(None)
            .await
            .expect("Failed to close connection");
    }
}
