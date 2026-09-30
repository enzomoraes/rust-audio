use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cpal::traits::{DeviceTrait, HostTrait};
use ringbuf::traits::{Consumer, Producer};
use ringbuf::{HeapCons, HeapProd};

use crate::pipeline::{Frame, Pipeline};

pub fn get_mic() -> cpal::Device {
    let host: cpal::Host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(default_device) => default_device,
        None => {
            panic!("Could not get default mic device")
        }
    };

    return device;
}

pub fn get_speaker() -> cpal::Device {
    let host: cpal::Host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(default_device) => default_device,
        None => {
            panic!("Could not get default speaker device")
        }
    };

    return device;
}

// ~10 ms per callback: low latency without waking the audio thread too often.
const TARGET_CALLBACK_MS: u32 = 10;
// Used when the backend can't report its buffer size range before the stream starts.
const FALLBACK_CALLBACK_FRAMES: usize = 2048;

pub struct DeviceConfig {
    pub stream: cpal::StreamConfig,
    pub callback_frames: usize,
}

pub fn input_config(device: &cpal::Device) -> DeviceConfig {
    match device.default_input_config() {
        Ok(supported) => with_fixed_buffer(&supported),
        Err(err) => {
            panic!(
                "Error getting input config, {:?} - {:?}",
                err.message(),
                err.kind()
            );
        }
    }
}

pub fn output_config(device: &cpal::Device) -> DeviceConfig {
    match device.default_output_config() {
        Ok(supported) => with_fixed_buffer(&supported),
        Err(err) => {
            panic!(
                "Error getting output config, {:?} - {:?}",
                err.message(),
                err.kind()
            );
        }
    }
}

fn with_fixed_buffer(supported: &cpal::SupportedStreamConfig) -> DeviceConfig {
    let mut stream = supported.config();
    let callback_frames = match *supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            // 10 ms: 480 frames at 48 kHz, 441 at 44.1 kHz, 960 at 96 kHz.
            let frames = (stream.sample_rate * TARGET_CALLBACK_MS / 1000).clamp(min, max);
            stream.buffer_size = cpal::BufferSize::Fixed(frames);
            frames as usize
        }
        cpal::SupportedBufferSize::Unknown => FALLBACK_CALLBACK_FRAMES,
    };
    DeviceConfig {
        stream,
        callback_frames,
    }
}

pub fn build_input_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut producer: HeapProd<Frame>,
    mut pipeline: Pipeline,
    dropped_counter: Arc<AtomicUsize>,
) -> cpal::Stream {
    let channels = config.channels as usize;

    let stream = match device.build_input_stream(
        config,
        move |data: &[f32], _info: &cpal::InputCallbackInfo| {
            // Mono mic is duplicated to both sides; channels beyond the first two are ignored.
            let frames = data.chunks(channels).map(|frame| match frame {
                [mono] => [*mono, *mono],
                [l, r, ..] => [*l, *r],
                [] => unreachable!(),
            });
            pipeline.process(frames, |out: &[Frame]| {
                let pushed = producer.push_slice(out);
                let dropped = out.len() - pushed;
                if dropped > 0 {
                    dropped_counter.fetch_add(dropped, Ordering::Relaxed);
                }
            });
        },
        move |err| eprintln!("{:?}", err),
        None,
    ) {
        Ok(stream) => stream,
        Err(err) => {
            panic!("Could not create input stream. {}", err)
        }
    };

    return stream;
}

pub fn build_output_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut consumer: HeapCons<Frame>,
) -> cpal::Stream {
    let channels = config.channels as usize;

    let stream = match device.build_output_stream(
        config,
        move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                // Empty buffer means silence.
                let [l, r] = consumer.try_pop().unwrap_or([0.0, 0.0]);
                match frame {
                    [mono] => *mono = (l + r) / 2.0,
                    [out_l, out_r, ..] => {
                        *out_l = l;
                        *out_r = r;
                    }
                    [] => {}
                }
            }
        },
        move |err: cpal::Error| eprintln!("{:?}", err),
        None,
    ) {
        Ok(stream) => stream,
        Err(err) => {
            panic!("Could not create output stream. {}", err)
        }
    };
    return stream;
}
