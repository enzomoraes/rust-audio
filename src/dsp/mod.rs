pub mod controls;
pub mod effects;

use std::sync::Arc;

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use controls::EffectControls;

pub type Frame = [f32; 2];

pub trait Effect: Send {
    fn controls(&self) -> &Arc<EffectControls>;

    /// Called once before audio starts, when the device's sample rate is known.
    fn prepare(&mut self, _sample_rate: u32) {}

    fn process(&mut self, block: &mut [Frame]);
}

/// Changes to the chain's structure, sent from the UI to the audio thread.
enum Command {
    Move { from: usize, to: usize },
}

// A burst of UI changes between two audio blocks never gets close to this.
const COMMAND_QUEUE: usize = 64;

/// The effect chain before audio starts: effects can be added freely here,
/// because allocating is still allowed.
#[derive(Default)]
pub struct PipelineBuilder {
    effects: Vec<Box<dyn Effect>>,
}

impl PipelineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(mut self, effect: impl Effect + 'static) -> Self {
        self.effects.push(Box::new(effect));
        self
    }

    /// Splits the chain into the part that runs on the audio thread and the
    /// handle the UI uses to control it.
    pub fn build(mut self, block_size: usize, sample_rate: u32) -> (Pipeline, PipelineRemote) {
        for effect in &mut self.effects {
            effect.prepare(sample_rate);
        }
        let controls = self
            .effects
            .iter()
            .map(|effect| effect.controls().clone())
            .collect();
        let (commands_tx, commands_rx) = HeapRb::new(COMMAND_QUEUE).split();

        let pipeline = Pipeline {
            effects: self.effects,
            data: vec![[0.0, 0.0]; block_size],
            commands: commands_rx,
        };
        let remote = PipelineRemote {
            effects: controls,
            commands: commands_tx,
        };
        (pipeline, remote)
    }
}

pub struct Pipeline {
    effects: Vec<Box<dyn Effect>>,
    data: Vec<Frame>,
    commands: HeapCons<Command>,
}

impl Pipeline {
    pub fn process(
        &mut self,
        mut input: impl Iterator<Item = Frame>,
        mut output: impl FnMut(&[Frame]),
    ) {
        self.apply_commands();

        loop {
            let n = self
                .data
                .iter_mut()
                .zip(input.by_ref())
                .map(|(pipeline_frame, input_frame)| *pipeline_frame = input_frame)
                .count();

            let block: &mut [Frame] = &mut self.data[..n];
            for effect in self.effects.iter_mut() {
                if effect.controls().is_enabled() {
                    effect.process(block);
                }
            }
            output(block);

            // A partially filled block means the input ran out.
            if n < self.data.len() {
                break;
            }
        }
    }

    fn apply_commands(&mut self) {
        while let Some(command) = self.commands.try_pop() {
            match command {
                // remove + insert reuse the Vec's existing memory: no allocation.
                Command::Move { from, to } => {
                    if from < self.effects.len() && to < self.effects.len() {
                        let effect = self.effects.remove(from);
                        self.effects.insert(to, effect);
                    }
                }
            }
        }
    }
}

/// The UI's side of the pipeline. It mirrors the effect order so the UI can draw
/// the chain without touching the audio thread.
pub struct PipelineRemote {
    effects: Vec<Arc<EffectControls>>,
    commands: HeapProd<Command>,
}

impl PipelineRemote {
    pub fn effects(&self) -> &[Arc<EffectControls>] {
        &self.effects
    }

    pub fn move_effect(&mut self, from: usize, to: usize) {
        if from == to || from >= self.effects.len() || to >= self.effects.len() {
            return;
        }
        // Only mirror the move locally if the audio thread will get it too,
        // otherwise the two orders would drift apart.
        if self.commands.try_push(Command::Move { from, to }).is_ok() {
            let effect = self.effects.remove(from);
            self.effects.insert(to, effect);
        }
    }
}
