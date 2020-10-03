use std::collections::VecDeque;

use audiopus::{Channels, SampleRate};
use audiopus::coder::Decoder;
use futures_util::core_reexport::time::Duration;
use serde::Deserialize;
use tokio::stream::StreamExt;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub struct AudioClient {
    decoder: Decoder,
    sender: Sender<Vec<f32>>,
    timestamp: u64,
    seconds: u64,
    buffer: VecDeque<Vec<f32>>,
    buffering: bool,
}

impl AudioClient {
    pub fn new(sender: Sender<Vec<f32>>) -> Self {
        AudioClient {
            decoder: Decoder::new(SampleRate::Hz48000, Channels::Stereo).unwrap(),
            sender,
            timestamp: 0,
            seconds: 0,
            buffer: VecDeque::with_capacity(50),
            buffering: true,
        }
    }
}

pub const SAMPLES_PER_FRAME: u64 = 960;
pub const SAMPLE_RATE: u64 = 48000;

#[derive(Deserialize)]
struct StreamStartMessage {
    timestamp: u64,
    name: String,
}

impl AudioClient {
    fn update_timestamp(&mut self, timestamp: u64) {
        // let old = self.seconds;
        self.timestamp = timestamp;
        self.seconds = timestamp / SAMPLE_RATE;
        // if old != self.seconds {
        //     info!("Timestamp: {}s, buffer: {}", self.seconds, self.buffer.len());
        // }
    }

    fn send(&mut self) {
        while let Some(data) = self.buffer.pop_front() {
            match self.sender.try_send(data) {
                Ok(_) => (),
                Err(e) => match e {
                    TrySendError::Full(data) => {
                        self.buffer.push_front(data);
                        break;
                    }
                    TrySendError::Closed(_) => panic!(),
                }
            }
        }
    }

    fn try_send(&mut self) {
        if self.buffering {
            self.buffering = self.buffer.len() < 50;
            if !self.buffering {
                info!("Finished buffering");
            }
        } else {
            self.send();
            self.buffering = self.buffer.is_empty();
            if self.buffering {
                info!("Buffering");
            }
        }
    }

    async fn handle_message(&mut self, message: tokio_tungstenite::tungstenite::Message) {
        match message {
            Message::Text(text) => {
                info!("Message: {}", &text);
                match serde_json::from_str::<StreamStartMessage>(&text) {
                    Ok(message) => {
                        info!("Playing \"{}\"", message.name);
                        self.update_timestamp(message.timestamp);
                    }
                    Err(err) => warn!("Invalid message, failed to parse: {}", err)
                }
            }
            Message::Binary(data) => {
                self.update_timestamp(self.timestamp + SAMPLES_PER_FRAME);
                let mut buffer = Vec::with_capacity(512 * 12);
                buffer.resize(512 * 12, 0.0);
                let res = self.decoder.decode_float(Some(data.as_slice()), buffer.as_mut_slice(), false).unwrap() * 2;
                buffer.resize(res, 0.0);
                self.buffer.push_back(buffer);
                self.try_send();
            }
            _ => ()
        }
    }

    pub async fn run(&mut self, address: String) {
        info!("Connecting to {}", &address);
        let (mut ws_stream, _) = connect_async(&address).await.unwrap();

        loop {
            match tokio::time::timeout(Duration::from_millis(20), ws_stream.next()).await {
                Ok(msg) => match msg {
                    Some(msg) => match msg {
                        Ok(msg) => self.handle_message(msg).await,
                        Err(_) => break,
                    }
                    None => break,
                }
                Err(_) => {
                    self.try_send();
                }
            }
        }

        ws_stream.close(None).await.expect("Failed to close connection");
    }
}
