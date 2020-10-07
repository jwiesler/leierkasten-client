use cpal::traits::{DeviceTrait, HostTrait};
use cpal::Stream;

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

/// Essentially an endless iterator, returning None means currently no data
pub trait AudioSource: Iterator<Item = Vec<f32>> + Send {}

pub fn create_stream<F: AudioSource + 'static>(mut source: F) -> Stream {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("failed to find a default output device");
    let config = device.default_output_config().unwrap();
    info!("Stream config: {:?}", config);

    let err_fn = |err| warn!("an error occurred on stream: {}", err);

    let mut current_chunk = None;

    let mut last_keep_up = true;

    // let callback = ;

    device
        .build_output_stream(
            &config.into(),
            move |mut data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                last_keep_up = loop {
                    if data.is_empty() {
                        break true;
                    }
                    let mut chunk = match current_chunk.take() {
                        None => match source.next() {
                            Some(chunk) => Chunk::new(chunk),
                            None => {
                                if last_keep_up {
                                    warn!("Can't keep up");
                                }
                                for x in data {
                                    *x = 0.0;
                                }
                                break false;
                            }
                        },
                        Some(chunk) => chunk,
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
                };
            },
            err_fn,
        )
        .unwrap()
}
