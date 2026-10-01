use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::DeviceTrait;
use ringbuf::traits::{Consumer, Producer};
use ringbuf::{HeapCons, HeapProd};

use crate::dsp::{Frame, Pipeline};
use crate::soundboard::Mixer;
use crate::telemetry::{Logger, Stats};

/// One place the processed audio goes. `active: None` means always on.
pub struct Destination {
    pub producer: HeapProd<Frame>,
    pub active: Option<Arc<AtomicBool>>,
}

pub fn build_input_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut destinations: Vec<Destination>,
    mut pipeline: Pipeline,
    mut mixer: Mixer,
    stats: Arc<Stats>,
    logger: Logger,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;

    device
        .build_input_stream(
            config,
            move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                stats.input_peak.store_max(peak(data.iter().copied()));

                // Mono mic is duplicated to both sides; channels beyond the first two are ignored.
                let frames = data.chunks(channels).map(|frame| match frame {
                    [mono] => [*mono, *mono],
                    [l, r, ..] => [*l, *r],
                    [] => unreachable!(),
                });
                pipeline.process(frames, |out: &mut [Frame]| {
                    // After the effects, so the sounds come out clean.
                    mixer.mix(out);
                    stats
                        .output_peak
                        .store_max(peak(out.iter().flatten().copied()));

                    for destination in destinations.iter_mut() {
                        let active = destination
                            .active
                            .as_ref()
                            .is_none_or(|active| active.load(Ordering::Relaxed));
                        if !active {
                            continue;
                        }
                        let pushed = destination.producer.push_slice(out);
                        let dropped = out.len() - pushed;
                        if dropped > 0 {
                            stats.dropped_frames.fetch_add(dropped, Ordering::Relaxed);
                        }
                    }
                });
            },
            move |err| logger.error(format!("mic stream error: {err}")),
            None,
        )
        .map_err(|err| format!("could not open the mic stream: {err}"))
}

/// Plays the processed audio while `active` is on, silence otherwise.
pub fn build_output_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut consumer: HeapCons<Frame>,
    active: Arc<AtomicBool>,
    logger: Logger,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;

    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                if !active.load(Ordering::Relaxed) {
                    // Throw away what was queued before switching off, so turning it
                    // back on doesn't replay old audio.
                    consumer.clear();
                    data.fill(0.0);
                    return;
                }
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
            move |err: cpal::Error| logger.error(format!("speaker stream error: {err}")),
            None,
        )
        .map_err(|err| format!("could not open the speaker stream: {err}"))
}

fn peak(samples: impl Iterator<Item = f32>) -> f32 {
    samples.fold(0.0, |max, sample| max.max(sample.abs()))
}
