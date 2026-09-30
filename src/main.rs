use cpal::traits::StreamTrait;
use ringbuf::HeapRb;
use ringbuf::traits::{Observer, Split};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use pipeline::{Gain, Pipeline};

use crate::pipeline::{RingModulation, SoftClipping};

mod io;
mod pipeline;

fn main() {
    let microphone = io::get_mic();
    let speaker = io::get_speaker();
    let mic_config = io::input_config(&microphone);
    let speaker_config = io::output_config(&speaker);
    let mic_rate = mic_config.stream.sample_rate;

    println!(
        "mic:     {} ch, {} Hz, {} frames/callback",
        mic_config.stream.channels, mic_rate, mic_config.callback_frames
    );
    println!(
        "speaker: {} ch, {} Hz, {} frames/callback",
        speaker_config.stream.channels,
        speaker_config.stream.sample_rate,
        speaker_config.callback_frames
    );
    if mic_rate != speaker_config.stream.sample_rate {
        eprintln!(
            "warning: mic and speaker sample rates differ; playback speed and pitch will be off"
        );
    }

    // 4x the largest callback: 480 frames at 48 kHz -> 1920 frames, ~40 ms max latency.
    let ring_capacity = mic_config
        .callback_frames
        .max(speaker_config.callback_frames)
        * 4;
    println!(
        "ring buffer: {} frames (max {:.1} ms of latency)",
        ring_capacity,
        ring_capacity as f32 / mic_rate as f32 * 1000.0
    );
    let (producer, consumer) = HeapRb::<pipeline::Frame>::new(ring_capacity).split();
    // Read-only view of the ring's indices, so the log thread can see occupancy.
    let ring_observer = consumer.observe();

    let pipeline = Pipeline::new(mic_config.callback_frames)
        .add(RingModulation::new(300.0, mic_rate))
        .add(Gain::new(10.0))
        .add(SoftClipping);
    let dropped_counter = Arc::new(AtomicUsize::new(0));

    // build output stream first so we don't fill the ring buffer while the speaker isn't ready to consume yet.
    let speaker_stream = io::build_output_stream(&speaker, speaker_config.stream, consumer);
    let mic_stream = io::build_input_stream(
        &microphone,
        mic_config.stream,
        producer,
        pipeline,
        dropped_counter.clone(),
    );

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));

            let queued = ring_observer.occupied_len();
            println!(
                "ring buffer: {queued}/{ring_capacity} frames ({:.1} ms)",
                queued as f32 / mic_rate as f32 * 1000.0
            );

            let dropped = dropped_counter.swap(0, Ordering::Relaxed);
            if dropped > 0 {
                let ms = dropped as f32 / mic_rate as f32 * 1000.0;
                eprintln!("dropped {dropped} frames ({ms:.1} ms) in the last second: buffer full");
            }
        }
    });

    mic_stream.play().unwrap();
    speaker_stream.play().unwrap();

    println!("Playing mic through speaker. Press Enter to stop.");
    let _ = std::io::stdin().read_line(&mut String::new());
}
