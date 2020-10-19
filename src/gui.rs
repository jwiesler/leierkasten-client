use std::collections::VecDeque;
use std::ffi::CString;
use std::iter::FromIterator;
use std::ops::Deref;
use std::sync::atomic::Ordering::Acquire;
use std::sync::atomic::Ordering::Release;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_util::core_reexport::sync::atomic::{AtomicU64, AtomicUsize};
use imgui::{Condition, ImStr, PlotLines, ProgressBar, Window};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::audio_client::{PlayingInfo, SAMPLE_RATE, TIME_BASE};
use crate::audio_socket::{AudioMessage, AudioSocket};
use crate::token::*;
use crate::{audio_socket, format, ADDRESS};

pub struct PlayerState {
    state: Mutex<PlayingInfo>,
    timestamp: AtomicU64,
    buffer: AtomicUsize,
    target_buffer: AtomicUsize,
}

impl PlayerState {
    pub fn new() -> Self {
        PlayerState {
            state: Mutex::new(PlayingInfo {
                item: None,
                buffering: true,
            }),
            timestamp: Default::default(),
            buffer: Default::default(),
            target_buffer: AtomicUsize::new(50),
        }
    }

    pub fn state(&self) -> MutexGuard<PlayingInfo> {
        self.state.lock().unwrap()
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp.load(Acquire)
    }

    pub fn buffer(&self) -> usize {
        self.buffer.load(Acquire)
    }

    pub fn target_buffer(&self) -> usize {
        self.target_buffer.load(Acquire)
    }

    pub fn set_timestamp(&self, timestamp: u64) {
        self.timestamp.store(timestamp, Release);
    }

    pub fn set_buffer(&self, buffer: usize) {
        self.buffer.store(buffer, Release);
    }

    pub fn set_target_buffer(&self, target_buffer: usize) {
        self.target_buffer.store(target_buffer, Release);
    }
}

pub type PlayerToken = Token<CancelableToken<CompletableToken<ValueToken<()>>>>;

pub struct Player {
    token: PlayerToken,
    player_state: Arc<PlayerState>,
    socket_state: Arc<Mutex<audio_socket::State>>,
    packet_output: Sender<AudioMessage>,
    handle: Option<JoinHandle<()>>,
    buffer_sizes: VecDeque<usize>,
}

impl Player {
    pub fn create_player(&self) -> JoinHandle<()> {
        let socket = AudioSocket::new(
            ADDRESS.into(),
            self.token.clone(),
            self.socket_state.clone(),
            self.packet_output.clone(),
        );
        tokio::spawn(async move { socket.run().await })
    }

    pub fn update(&mut self) {
        let handle = self.handle.take();
        self.handle = match handle {
            None => None,
            Some(handle) => {
                if self.token.is_completed() {
                    self.token.reset();
                    *self.socket_state.lock().unwrap() = audio_socket::State::None;
                    None
                } else {
                    Some(handle)
                }
            }
        };
    }

    pub fn build(&mut self, ui: &imgui::Ui) {
        self.update();
        match self.socket_state.lock().unwrap().deref() {
            audio_socket::State::None => {
                ui.text(im_str!("Not connected"));
                if ui.button(im_str!("Connect"), [0.0, 0.0]) {
                    self.handle = Some(self.create_player());
                }
            }
            audio_socket::State::Connecting => {
                ui.text(im_str!("Connecting..."));
            }
            audio_socket::State::Connected => {
                let mut info = self.player_state.state.lock().unwrap();
                if self.token.is_canceled() {
                    ui.text(im_str!("Disconnecting..."));
                } else {
                    let timestamp_us = self.player_state.timestamp() * TIME_BASE / SAMPLE_RATE;

                    struct Current<'a> {
                        name: &'a str,
                        timestamp: i64,
                        duration_s: Option<i64>,
                    }

                    let (current, progress) = match info.item.as_ref() {
                        Some(item) => {
                            let timestamp_us = item.start_timestamp_us + timestamp_us;
                            let (duration_s, progress) = match item.end_timestamp_us.clone() {
                                Some(end_timestamp_us) => (
                                    Some((end_timestamp_us / TIME_BASE) as i64),
                                    timestamp_us as f32 / end_timestamp_us as f32,
                                ),
                                None => (None, 0.0),
                            };
                            let current = Current {
                                name: item.name.as_str(),
                                timestamp: (timestamp_us / TIME_BASE) as i64,
                                duration_s,
                            };
                            (Some(current), progress)
                        }
                        None => (None, 0.0),
                    };

                    ui.text("Title:");
                    ui.same_line(0.0);
                    if let Some(current) = current.as_ref() {
                        unsafe {
                            ui.text_wrapped(ImStr::from_cstr_unchecked(
                                &CString::new(current.name.as_bytes()).unwrap(),
                            ));
                        }
                    } else {
                        ui.text(im_str!("-"));
                    }

                    ui.text(im_str!("Timestamp:"));
                    ui.same_line(0.0);
                    if let Some(current) = current.as_ref() {
                        ui.text(format::format_timestamp(current.timestamp));

                        if let Some(duration) = current.duration_s {
                            ui.same_line(0.0);
                            ui.text(im_str!("-"));
                            ui.same_line(0.0);
                            ui.text(format::format_timestamp(duration));
                        }
                    } else {
                        ui.text(im_str!("--:--"));
                    }

                    if info.buffering {
                        ui.same_line_with_spacing(0.0, 20.0);
                        ui.text(im_str!("Buffering"));
                    }

                    ui.spacing();

                    ProgressBar::new(progress)
                        .overlay_text(im_str!(""))
                        .build(ui);
                    if ui.button(im_str!("Disconnect"), [0.0, 0.0]) {
                        self.token.cancel();
                        info.item = None;
                        info.buffering = true;
                    }

                    {
                        let new_buffer = self.player_state.buffer();
                        let _ = self.buffer_sizes.pop_front();
                        self.buffer_sizes.push_back(new_buffer);
                    }

                    fn buffer_len_to_ms(buffer: usize) -> f32 {
                        (buffer * 960000) as f32 / SAMPLE_RATE as f32
                    }

                    let max = *self.buffer_sizes.iter().max().unwrap();

                    if imgui::CollapsingHeader::new(im_str!("Advanced")).build(ui) {
                        let buffer = self
                            .buffer_sizes
                            .iter()
                            .map(|b| buffer_len_to_ms(*b))
                            .collect::<Vec<f32>>();
                        PlotLines::new(ui, im_str!("Buffer [ms]"), buffer.as_slice())
                            .graph_size([0.0, 80.0])
                            .scale_min(0.0)
                            .scale_max(buffer_len_to_ms(max) as f32)
                            .build();
                    }
                }
            }
            audio_socket::State::Disconnecting => {
                ui.text(im_str!("Disconnecting..."));
            }
        }
    }
}

pub struct GuiState {
    player: Player,
}

impl GuiState {
    pub fn new(packet_output: Sender<AudioMessage>, player_state: Arc<PlayerState>) -> Self {
        GuiState {
            player: Player {
                token: PlayerToken::default(),
                packet_output,
                player_state,
                handle: None,
                socket_state: Arc::new(Mutex::new(audio_socket::State::None)),
                buffer_sizes: VecDeque::from_iter(std::iter::repeat(0).take(10 * 1000 / 20)),
            },
        }
    }

    pub fn build(&mut self, ui: &mut imgui::Ui) {
        Window::new(im_str!("Player"))
            .size([400.0, 150.0], Condition::FirstUseEver)
            .build(ui, || self.player.build(ui));
    }
}
