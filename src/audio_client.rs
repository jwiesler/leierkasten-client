use std::collections::VecDeque;
use std::ops::DerefMut;
use std::sync::Arc;

use audiopus::coder::Decoder;
use audiopus::{Channels, SampleRate};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::Receiver;

use crate::audio_socket::AudioMessage;
use crate::audio_socket::StreamStartMessage;
use crate::audio_stream::AudioSource;
use crate::gui::PlayerState;

pub struct PlayingInfo {
    pub item: Option<StreamStartMessage>,
    pub buffering: bool,
}

pub struct AudioClient {
    decoder: Decoder,
    timestamp: u64,
    buffer: VecDeque<AudioMessage>,
    buffering: bool,
    receiver: Receiver<AudioMessage>,
    context: Arc<PlayerState>,
}

impl AudioClient {
    pub fn new(receiver: Receiver<AudioMessage>, context: Arc<PlayerState>) -> Self {
        AudioClient {
            decoder: Decoder::new(SampleRate::Hz48000, Channels::Stereo).unwrap(),
            receiver,
            timestamp: 0,
            buffer: VecDeque::with_capacity(50),
            buffering: true,
            context,
        }
    }
}

pub const SAMPLES_PER_FRAME: u64 = 960;
pub const SAMPLE_RATE: u64 = 48000;
pub const TIME_BASE: u64 = 1000000;

impl AudioClient {
    fn update_timestamp(&mut self, timestamp: u64) {
        self.timestamp = timestamp;
        self.context.set_timestamp(self.timestamp);
    }

    fn set_context_buffering(&mut self) {
        self.context.state().deref_mut().buffering = self.buffering;
        if self.buffering {
            info!("Buffering");
        } else {
            info!("Finished buffering");
        }
    }

    fn decode(&mut self, data: Vec<u8>) -> Vec<f32> {
        self.update_timestamp(self.timestamp + SAMPLES_PER_FRAME);
        let mut buffer = Vec::with_capacity(512 * 12);
        buffer.resize(512 * 12, 0.0);
        let res = self
            .decoder
            .decode_float(Some(data.as_slice()), buffer.as_mut_slice(), false)
            .unwrap()
            * 2;
        buffer.resize(res, 0.0);
        buffer
    }

    fn handle_new_resource(&mut self, message: StreamStartMessage) {
        let offset_sample = message.offset_samples;
        *self.context.state() = PlayingInfo {
            item: Some(message),
            buffering: self.buffering,
        };
        self.update_timestamp(offset_sample);
    }

    fn decode_one(&mut self) -> Option<Vec<f32>> {
        if self.buffering {
            return None;
        }
        loop {
            match self.buffer.pop_front() {
                None => return None,
                Some(message) => {
                    self.context.set_buffer(self.buffer.len());
                    if self.buffer.is_empty() {
                        self.buffering = true;
                        self.set_context_buffering();
                    }
                    match message {
                        AudioMessage::NewResource(info) => self.handle_new_resource(info),
                        AudioMessage::Audio(data) => return Some(self.decode(data)),
                    }
                }
            }
        }
    }

    fn receive_all(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(m) => {
                    self.buffer.push_back(m);
                    self.context.set_buffer(self.buffer.len());
                    if self.buffering {
                        self.buffering = self.buffer.len() < self.context.target_buffer();
                        if !self.buffering {
                            self.set_context_buffering();
                        }
                    }
                }
                Err(e) => match e {
                    TryRecvError::Empty => break,
                    TryRecvError::Closed => panic!(),
                },
            }
        }
    }
}

impl Iterator for AudioClient {
    type Item = Vec<f32>;

    fn next(&mut self) -> Option<Vec<f32>> {
        self.receive_all();
        self.decode_one()
    }
}

impl AudioSource for AudioClient {}
