mod devices;
mod streams;
#[cfg(target_os = "linux")]
mod virtual_mic;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::StreamTrait;
use ringbuf::traits::{Observer, Split};
use ringbuf::{HeapRb, Obs};

use crate::dsp::{Frame, PipelineBuilder, PipelineRemote};
use crate::telemetry::{Logger, Stats};
use streams::Destination;

// PipeWire picks the virtual mic's block size itself; a generous guess for sizing the ring.
#[cfg(target_os = "linux")]
const VIRTUAL_MIC_BLOCK_FRAMES: usize = 2048;

pub struct EngineInfo {
    pub input_device: String,
    /// `None` when the speakers couldn't be opened, so there's nothing to listen on.
    pub speaker_device: Option<String>,
    pub has_virtual_mic: bool,
    pub sample_rate: u32,
    pub block_frames: usize,
}

pub struct QueueStatus {
    pub name: &'static str,
    pub queued: usize,
    pub capacity: usize,
}

/// One ring buffer between the mic callback and an output.
struct Queue {
    name: &'static str,
    capacity: usize,
    observer: Obs<Arc<HeapRb<Frame>>>,
}

/// Mic -> pipeline -> the Rust Phone virtual mic, and to the speakers while
/// "listen to myself" is on. Audio runs for as long as this value is alive.
pub struct Engine {
    info: EngineInfo,
    stats: Arc<Stats>,
    monitoring: Arc<AtomicBool>,
    queues: Vec<Queue>,
    _input: cpal::Stream,
    _speaker: Option<cpal::Stream>,
}

impl Engine {
    pub fn start(
        chain: PipelineBuilder,
        logger: Logger,
    ) -> Result<(Engine, PipelineRemote), String> {
        let mic = devices::default_mic()?;
        let mic_config = devices::input_config(&mic)?;
        let sample_rate = mic_config.stream.sample_rate;
        let block_frames = mic_config.callback_frames;

        let has_virtual_mic = cfg!(target_os = "linux");
        let speaker = match devices::default_speaker()
            .and_then(|device| devices::output_config(&device).map(|config| (device, config)))
        {
            Ok((device, config)) => {
                if config.stream.sample_rate != sample_rate {
                    logger.warn(
                        "mic and speaker sample rates differ; listening to yourself will sound off",
                    );
                }
                Some((device, config))
            }
            Err(err) => {
                logger.warn(format!(
                    "speakers unavailable, can't listen to yourself: {err}"
                ));
                None
            }
        };
        if !has_virtual_mic && speaker.is_none() {
            return Err("no output available: no virtual mic on this OS and no speakers".into());
        }

        let stats = Arc::new(Stats::default());
        let monitoring = Arc::new(AtomicBool::new(false));
        let (pipeline, remote) = chain.build(block_frames, sample_rate);
        let mut destinations = Vec::new();
        let mut queues = Vec::new();

        let info = EngineInfo {
            input_device: devices::name(&mic),
            speaker_device: speaker.as_ref().map(|(device, _)| devices::name(device)),
            has_virtual_mic,
            sample_rate,
            block_frames,
        };

        // Outputs first, so they're ready to consume before the mic starts filling them.
        let speaker_stream = match speaker {
            Some((device, config)) => {
                // 4x the largest callback: 480 frames at 48 kHz -> 1920 frames, ~40 ms max latency.
                let capacity = block_frames.max(config.callback_frames) * 4;
                let (producer, consumer) = HeapRb::<Frame>::new(capacity).split();
                queues.push(Queue {
                    name: "Speakers",
                    capacity,
                    observer: consumer.observe(),
                });
                destinations.push(Destination {
                    producer,
                    active: Some(monitoring.clone()),
                });
                Some(streams::build_output_stream(
                    &device,
                    config.stream,
                    consumer,
                    monitoring.clone(),
                    logger.clone(),
                )?)
            }
            None => None,
        };

        #[cfg(target_os = "linux")]
        {
            let capacity = block_frames.max(VIRTUAL_MIC_BLOCK_FRAMES) * 4;
            let (producer, consumer) = HeapRb::<Frame>::new(capacity).split();
            queues.push(Queue {
                name: "Rust Phone mic",
                capacity,
                observer: consumer.observe(),
            });
            destinations.push(Destination {
                producer,
                active: None,
            });
            virtual_mic::spawn(consumer, sample_rate, block_frames, logger.clone());
        }

        let input = streams::build_input_stream(
            &mic,
            mic_config.stream,
            destinations,
            pipeline,
            stats.clone(),
            logger.clone(),
        )?;

        input
            .play()
            .map_err(|err| format!("could not start the mic: {err}"))?;
        if let Some(stream) = &speaker_stream {
            stream
                .play()
                .map_err(|err| format!("could not start the speakers: {err}"))?;
        }

        logger.info(format!(
            "Audio running from {} ({} Hz, {} frames/block)",
            info.input_device, info.sample_rate, info.block_frames
        ));

        let engine = Engine {
            info,
            stats,
            monitoring,
            queues,
            _input: input,
            _speaker: speaker_stream,
        };
        Ok((engine, remote))
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn is_monitoring(&self) -> bool {
        self.monitoring.load(Ordering::Relaxed)
    }

    pub fn set_monitoring(&self, on: bool) {
        self.monitoring.store(on, Ordering::Relaxed);
    }

    pub fn queues(&self) -> impl Iterator<Item = QueueStatus> + '_ {
        self.queues.iter().map(|queue| QueueStatus {
            name: queue.name,
            queued: queue.observer.occupied_len(),
            capacity: queue.capacity,
        })
    }
}
