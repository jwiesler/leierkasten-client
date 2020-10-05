#[macro_use]
extern crate imgui;
#[macro_use]
extern crate log;

mod audio_client;
mod audio_stream;
mod single_buffer_sender;
mod token;

use token::*;

use std::ops::Deref;
use std::sync::atomic::Ordering::Acquire;
use std::sync::atomic::Ordering::Release;
use std::sync::{Arc, Mutex, MutexGuard};

use cpal::traits::StreamTrait;
use futures_util::core_reexport::sync::atomic::{AtomicU64, AtomicUsize};
use imgui::{Condition, ProgressBar, Window};
use num_integer::Integer;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::audio_client::{AudioClient, SAMPLE_RATE};
use crate::audio_stream::create_stream;

mod gfx_system;

pub enum State {
    None,
    Connecting,
    Buffering,
    Preparing,
    Playing(Box<PlayingInfo>),
}

pub struct PlayingInfo {
    pub name: String,
    pub buffering: bool,
    pub duration: Option<u64>,
}

pub struct PlayerContext {
    state: Mutex<State>,
    timestamp: AtomicU64,
    buffer: AtomicUsize,
}

impl PlayerContext {
    pub fn new() -> Self {
        PlayerContext {
            state: Mutex::new(State::None),
            timestamp: Default::default(),
            buffer: Default::default(),
        }
    }

    pub fn state(&self) -> MutexGuard<State> {
        self.state.lock().unwrap()
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp.load(Acquire)
    }

    pub fn buffer(&self) -> usize {
        self.buffer.load(Acquire)
    }

    pub fn set_timestamp(&self, timestamp: u64) {
        self.timestamp.store(timestamp, Release);
    }

    pub fn set_buffer(&self, buffer: usize) {
        self.buffer.store(buffer, Release);
    }
}

pub type PlayerToken = Token<CancelableToken<CompletableToken<ValueToken<()>>>>;

pub struct Player {
    token: PlayerToken,
    audio_connection: Sender<Vec<f32>>,
    context: Arc<PlayerContext>,
    handle: Option<JoinHandle<()>>,
}

impl Player {
    pub fn create_player(&self) -> JoinHandle<()> {
        let mut client = AudioClient::new(self.audio_connection.clone(), self.context.clone());
        let token = self.token.clone();
        tokio::spawn(async move { client.run(ADDRESS.into(), token).await })
    }

    pub fn update(&mut self) {
        let handle = self.handle.take();
        self.handle = match handle {
            None => None,
            Some(handle) => {
                if self.token.is_completed() {
                    self.token.reset();
                    *self.context.state() = State::None;
                    None
                } else {
                    Some(handle)
                }
            }
        };
    }
}

pub struct GuiState {
    player: Player,
}

impl GuiState {
    pub fn new(audio_connection: Sender<Vec<f32>>) -> Self {
        GuiState {
            player: Player {
                token: PlayerToken::default(),
                audio_connection,
                context: Arc::new(PlayerContext::new()),
                handle: None,
            },
        }
    }
}

fn push_digit(res: &mut String, num: i64) {
    debug_assert!(num >= 0 && num < 10);
    res.push((b'0' + num as u8) as char);
}

fn format_timestamp(seconds: i64) -> String {
    let mut res = String::with_capacity(8);

    let (hours, seconds) = seconds.div_rem(&3600);
    if hours > 0 {
        let (tens, units) = hours.div_rem(&10);
        if tens > 0 {
            push_digit(&mut res, tens);
        }
        push_digit(&mut res, units);
        res += ":";
    }

    let (minutes, seconds) = seconds.div_rem(&60);
    let (tens, units) = minutes.div_rem(&10);
    if hours > 0 || tens != 0 {
        push_digit(&mut res, tens);
    }
    push_digit(&mut res, units);
    res += ":";

    let (tens, units) = seconds.div_rem(&10);
    push_digit(&mut res, tens);
    push_digit(&mut res, units);
    res
}

async fn run_gui(mut state: GuiState) {
    let system = gfx_system::init("Leierkasten Client");
    system
        .main_loop(|_, ui| {
            Window::new(im_str!("Player"))
                .size([400.0, 150.0], Condition::FirstUseEver)
                .build(ui, || {
                    state.player.update();
                    match state.player.context.state().deref() {
                        State::None => {
                            ui.text(im_str!("Not connected"));
                            ui.same_line(0.0);
                            if ui.small_button(im_str!("Connect")) {
                                state.player.handle = Some(state.player.create_player());
                            }
                        }
                        State::Connecting => {
                            ui.text(im_str!("Connecting..."));
                        }
                        State::Buffering => {
                            ui.text(im_str!("Buffering..."));
                        }
                        State::Preparing => {
                            ui.text(im_str!("Preparing..."));
                        }
                        State::Playing(info) => {
                            if state.player.token.is_canceled() {
                                ui.text(im_str!("Disconnecting..."));
                            } else {
                                let timestamp = state.player.context.timestamp();
                                ui.text(im_str!("Title:"));
                                ui.same_line(0.0);
                                ui.text(&info.name);

                                ui.text(im_str!("Timestamp:"));
                                ui.same_line(0.0);
                                ui.text(format_timestamp((timestamp / SAMPLE_RATE) as i64));
                                let progress = if let Some(duration) = info.duration.clone() {
                                    ui.same_line(0.0);
                                    ui.text(im_str!("-"));
                                    ui.same_line(0.0);
                                    ui.text(format_timestamp((duration / SAMPLE_RATE) as i64));
                                    timestamp as f32 / duration as f32
                                } else {
                                    0.0
                                };

                                ui.same_line_with_spacing(0.0, 20.0);
                                if info.buffering {
                                    ui.text(im_str!("Buffering"));
                                }

                                ui.spacing();

                                ProgressBar::new(progress)
                                    .overlay_text(im_str!(""))
                                    .build(ui);
                                if ui.small_button(im_str!("Disconnect")) {
                                    state.player.token.cancel();
                                }

                                ui.text(im_str!("Buffer:"));
                                ui.same_line(0.0);
                                ui.text(state.player.context.buffer().to_string());
                            }
                        }
                    }
                });
        })
        .await;
}

const ADDRESS: &str = "ws://localhost:2020/";

#[tokio::main(core_threads = 4)]
async fn main() -> Result<(), std::io::Error> {
    let _ =
        std::env::var("RUST_LOG").map_err(|_| std::env::set_var("RUST_LOG", "leierkasten_client"));
    env_logger::init();

    let (sender, receiver) = tokio::sync::mpsc::channel(5);
    let stream = create_stream(receiver);

    let context = GuiState::new(sender.clone());

    info!("Playing stream");
    stream.play().unwrap();

    run_gui(context).await;

    info!("Exiting");
    Ok(())
}
