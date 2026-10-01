mod audio;
mod dsp;
mod soundboard;
mod telemetry;
mod ui;

use std::time::Instant;

use audio::Engine;
use dsp::PipelineBuilder;
use dsp::effects::{Gain, NoiseSuppression, RingModulation, SoftClipping};

fn main() -> eframe::Result {
    let started = Instant::now();
    let (logger, logs) = telemetry::log_channel();

    // Noise removal first, so the robot effects work on the cleaned voice.
    let chain = PipelineBuilder::new()
        .add(NoiseSuppression::new())
        .add(RingModulation::new(200.0))
        .add(Gain::new(10.0))
        .add(SoftClipping::new());

    let engine = Engine::start(chain, logger.clone());
    if let Err(err) = &engine {
        logger.error(format!("audio could not start: {err}"));
    }

    ui::run(engine, logs, logger, started)
}
