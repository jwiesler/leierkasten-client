#[macro_use]
extern crate imgui;
#[macro_use]
extern crate log;

use crate::audio_client::AudioClient;
use crate::audio_stream::create_stream;
use crate::gui::{GuiState, PlayerState};
use cpal::traits::StreamTrait;
use std::sync::Arc;

mod audio_client;
mod audio_socket;
mod audio_stream;
mod format;
mod gfx_system;
mod gui;
mod single_buffer_sender;
mod token;

async fn run_gui(mut state: GuiState) {
    let system = gfx_system::init("Leierkasten Client");
    system.main_loop(|_, ui| state.build(ui)).await;
}

const ADDRESS: &str = "ws://localhost:2020/";

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let _ =
        std::env::var("RUST_LOG").map_err(|_| std::env::set_var("RUST_LOG", "leierkasten_client"));
    env_logger::init();

    let (sender, receiver) = tokio::sync::mpsc::channel(5);
    let state = Arc::new(PlayerState::new());
    let client = AudioClient::new(receiver, state.clone());
    let stream = create_stream(client);

    let context = GuiState::new(sender, state);

    info!("Playing stream");
    stream.play().unwrap();

    run_gui(context).await;

    info!("Exiting");
    Ok(())
}
