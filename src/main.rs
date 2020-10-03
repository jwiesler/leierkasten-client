#[macro_use]
extern crate log;

use cpal::Stream;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc::Receiver;

use crate::audio_client::AudioClient;

mod audio_client;

struct Chunk {
    data: Vec<f32>,
    offset: usize,
}

impl Chunk {
    pub fn new(data: Vec<f32>) -> Self {
        Chunk { data, offset: 0 }
    }

    pub fn remaining_slice(&self) -> &[f32] {
        &self.data[self.offset..]
    }
}

fn create_stream(mut receiver: Receiver<Vec<f32>>) -> Stream {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("failed to find a default output device");
    let config = device.default_output_config().unwrap();
    info!("Stream config: {:?}", config);

    let err_fn = |err| warn!("an error occurred on stream: {}", err);

    let mut current_chunk = None;

    let mut last_keep_up = true;
    device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut data = data;
            while !data.is_empty() {
                let mut chunk = match current_chunk.take() {
                    None => match receiver.try_recv() {
                        Ok(chunk) => {
                            Chunk::new(chunk)
                        }
                        Err(_) => {
                            if last_keep_up {
                                last_keep_up = false;
                                warn!("Can't keep up");
                            }
                            for x in data {
                                *x = 0.0;
                            }
                            return;
                        }
                    }
                    Some(chunk) => chunk
                };

                let remaining_data = chunk.remaining_slice();
                let split_point = remaining_data.len().min(data.len());
                let (a, new_data) = data.split_at_mut(split_point);
                a.copy_from_slice(&remaining_data[..split_point]);
                if split_point < remaining_data.len() {
                    chunk.offset += split_point;
                    current_chunk = Some(chunk);
                }
                data = new_data;
            }
            last_keep_up = true;
        },
        err_fn).unwrap()
}

#[tokio::main(core_threads = 4)]
async fn main() -> Result<(), std::io::Error> {
    let _ = std::env::var("RUST_LOG").map_err(|_| std::env::set_var("RUST_LOG", "leierkasten_client"));
    env_logger::init();

    let (sender, receiver) = tokio::sync::mpsc::channel(5);
    let stream = create_stream(receiver);

    info!("Playing stream");
    stream.play().unwrap();

    let mut audio = AudioClient::new(sender);
    let t = tokio::spawn(async move {
        audio.run("ws://localhost:2020/".into()).await;
    });

    t.await.unwrap();

    info!("Exiting");
    Ok(())
}
