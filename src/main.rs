mod audio;
mod dsp;
mod soundboard;
mod telemetry;
mod ui;

use std::time::Instant;

use audio::Engine;
use dsp::PipelineBuilder;
use dsp::effects::{Gain, NoiseGate, RingModulation, SoftClipping};

fn main() -> eframe::Result {
    let started = Instant::now();
    let (logger, logs) = telemetry::log_channel();

    // The gate first: it has to judge the raw mic level, before gain changes it.
    let chain = PipelineBuilder::new()
        .add(NoiseGate::new(-40.0))
        .add(RingModulation::new(200.0))
        .add(Gain::new(10.0))
        .add(SoftClipping::new());

    let engine = Engine::start(chain, logger.clone());
    if let Err(err) = &engine {
        logger.error(format!("audio could not start: {err}"));
    }

    ui::run(engine, logs, logger, started)
}
