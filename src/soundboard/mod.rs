//! Plays audio clips into the outputs on top of the processed voice, untouched by the
//! effects. Decoding happens off the audio thread; the audio thread only ever gets
//! ready-made frames behind an `Arc`, and hands each one back when it's done so the
//! memory is never freed inside the audio callback.

pub mod decode;
pub mod library;

use std::sync::Arc;

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use crate::dsp::Frame;
use crate::dsp::controls::Param;

// How many sounds can play at the same time.
const MAX_VOICES: usize = 8;
const COMMAND_QUEUE: usize = 32;
// Room for every voice plus every pending Play to come back at once.
const RETURN_QUEUE: usize = MAX_VOICES + COMMAND_QUEUE;

/// A clip decoded at the engine's sample rate.
pub struct Sound {
    pub frames: Vec<Frame>,
}

enum Command {
    Play(Arc<Sound>),
    StopAll,
}

struct Voice {
    sound: Arc<Sound>,
    position: usize,
}

pub fn channel() -> (Mixer, SoundboardRemote) {
    let (commands_tx, commands_rx) = HeapRb::new(COMMAND_QUEUE).split();
    let (returns_tx, returns_rx) = HeapRb::new(RETURN_QUEUE).split();
    let volume = Arc::new(Param::new("Volume", "%", 0.0, 150.0, 100.0));

    let mixer = Mixer {
        voices: Vec::with_capacity(MAX_VOICES),
        commands: commands_rx,
        returns: returns_tx,
        volume: volume.clone(),
    };
    let remote = SoundboardRemote {
        commands: commands_tx,
        returns: returns_rx,
        volume,
    };
    (mixer, remote)
}

/// The audio thread's side: adds every playing clip into each output block.
pub struct Mixer {
    voices: Vec<Voice>,
    commands: HeapCons<Command>,
    returns: HeapProd<Arc<Sound>>,
    volume: Arc<Param>,
}

impl Mixer {
    pub fn mix(&mut self, block: &mut [Frame]) {
        while let Some(command) = self.commands.try_pop() {
            match command {
                Command::Play(sound) => {
                    // `voices` was allocated with room for MAX_VOICES, so this push never allocates.
                    if self.voices.len() < MAX_VOICES {
                        self.voices.push(Voice { sound, position: 0 });
                    } else {
                        self.give_back(sound);
                    }
                }
                Command::StopAll => {
                    while let Some(voice) = self.voices.pop() {
                        self.give_back(voice.sound);
                    }
                }
            }
        }

        let gain = self.volume.get() / 100.0;
        let mut index = 0;
        while index < self.voices.len() {
            let voice = &mut self.voices[index];
            let remaining = &voice.sound.frames[voice.position..];
            for (out, sample) in block.iter_mut().zip(remaining) {
                out[0] += sample[0] * gain;
                out[1] += sample[1] * gain;
            }
            voice.position += block.len().min(remaining.len());

            if voice.position >= voice.sound.frames.len() {
                let voice = self.voices.swap_remove(index);
                self.give_back(voice.sound);
            } else {
                index += 1;
            }
        }
    }

    fn give_back(&mut self, sound: Arc<Sound>) {
        // Only fails if the UI stopped draining; dropping here is the rare fallback.
        let _ = self.returns.try_push(sound);
    }
}

/// The UI's side.
pub struct SoundboardRemote {
    commands: HeapProd<Command>,
    returns: HeapCons<Arc<Sound>>,
    volume: Arc<Param>,
}

impl SoundboardRemote {
    /// Returns false if the command queue was full and the sound won't play.
    pub fn play(&mut self, sound: &Arc<Sound>) -> bool {
        self.commands.try_push(Command::Play(sound.clone())).is_ok()
    }

    pub fn stop_all(&mut self) {
        let _ = self.commands.try_push(Command::StopAll);
    }

    pub fn volume(&self) -> &Param {
        &self.volume
    }

    /// Sounds the mixer finished with since the last call.
    pub fn finished(&mut self) -> impl Iterator<Item = Arc<Sound>> + '_ {
        std::iter::from_fn(|| self.returns.try_pop())
    }
}
