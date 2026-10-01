use std::f32::consts::TAU;
use std::sync::Arc;

use nnnoiseless::DenoiseState;

use super::controls::{EffectControls, Param};
use super::{Effect, Frame};

pub struct Gain {
    controls: Arc<EffectControls>,
}

impl Gain {
    /// Creates a gain from a value in decibels.
    ///
    /// A sample is the speaker cone's position (its amplitude), and the sound's
    /// power grows with the square of that amplitude (P ∝ A²). Decibels are
    /// defined over power, so for amplitude the square turns into a factor of 2:
    ///
    ///   dB = 10·log10(P/P0) = 10·log10(A²/A0²) = 20·log10(A/A0)
    ///
    /// Solving for the amplitude ratio gives the factor each sample is multiplied by:
    ///
    ///   factor = 10^(dB/20)
    ///
    /// +20 dB -> x10, +6 dB -> ~x2, 0 dB -> x1 (unchanged), -6 dB -> ~x0.5.
    pub fn new(db: f32) -> Self {
        Self {
            controls: Arc::new(EffectControls::new(
                "Gain",
                "Makes the signal louder or quieter.",
                vec![Param::new("Gain", " dB", -24.0, 24.0, db)],
            )),
        }
    }
}

impl Effect for Gain {
    fn controls(&self) -> &Arc<EffectControls> {
        &self.controls
    }

    fn process(&mut self, block: &mut [Frame]) {
        let factor = 10f32.powf(self.controls.params[0].get() / 20.0);
        for frame in block.iter_mut() {
            for sample in frame.iter_mut() {
                *sample *= factor;
            }
        }
    }
}

/// Bends loud samples smoothly toward ±1.0 instead of cutting them off.
///
/// Samples above ±1.0 can't be played: the hardware flattens them (hard clipping),
/// and the sharp corners that creates sound harsh. `tanh` maps any input into
/// (-1, 1) with a smooth curve instead:
///
/// - near zero it's almost linear (tanh(x) ≈ x), so quiet sounds pass unchanged;
/// - as |x| grows it flattens gradually, so loud peaks are squeezed, never cut.
///
/// The rounded shape adds a warm distortion (like overdriven tubes or tape)
/// rather than a harsh one.
///
///   input:  0.1     0.5     1.0     2.0     3.0     10.0
///   tanh:   0.100   0.462   0.762   0.964   0.995   1.000
///   hard:   0.1     0.5     1.0     1.0     1.0     1.0
///
/// Placed after a `Gain`, it behaves like a distortion pedal: the more gain,
/// the more of the signal lands in the curved part and the more it saturates.
///
///   PipelineBuilder::new()
///       .add(Gain::new(20.0))
///       .add(SoftClipping::new())
pub struct SoftClipping {
    controls: Arc<EffectControls>,
}

impl SoftClipping {
    pub fn new() -> Self {
        Self {
            controls: Arc::new(EffectControls::new(
                "Soft clipping",
                "Rounds off peaks above ±1.0 instead of cutting them.",
                vec![],
            )),
        }
    }
}

impl Effect for SoftClipping {
    fn controls(&self) -> &Arc<EffectControls> {
        &self.controls
    }

    fn process(&mut self, block: &mut [Frame]) {
        for frame in block.iter_mut() {
            for sample in frame.iter_mut() {
                *sample = sample.tanh();
            }
        }
    }
}

/// Multiplies the signal by a sine carrier, turning every frequency f into
/// f - carrier and f + carrier. The voice's harmonics stop being integer
/// multiples of one pitch, so it sounds metallic and robotic.
///
///   ~30 Hz: buzzy Dalek voice | 100-300 Hz: metallic, bell-like
pub struct RingModulation {
    controls: Arc<EffectControls>,
    sample_rate: f32,
    phase: f32,
}

impl RingModulation {
    pub fn new(frequency: f32) -> Self {
        Self {
            controls: Arc::new(EffectControls::new(
                "Ring modulation",
                "Multiplies the voice by a sine wave: the robot sound.",
                vec![Param::new("Carrier", " Hz", 10.0, 1000.0, frequency)],
            )),
            sample_rate: 48_000.0,
            phase: 0.0,
        }
    }
}

impl Effect for RingModulation {
    fn controls(&self) -> &Arc<EffectControls> {
        &self.controls
    }

    fn prepare(&mut self, sample_rate: u32) -> Result<(), String> {
        self.sample_rate = sample_rate as f32;
        Ok(())
    }

    fn process(&mut self, block: &mut [Frame]) {
        // Recomputed per block so the frequency can change live without a click:
        // the phase just keeps going at the new speed.
        let step = TAU * self.controls.params[0].get() / self.sample_rate;
        for frame in block.iter_mut() {
            // One carrier value per frame, so L and R stay in sync.
            let carrier = self.phase.sin();
            for sample in frame.iter_mut() {
                *sample *= carrier;
            }

            self.phase += step;
            if self.phase >= TAU {
                self.phase -= TAU;
            }
        }
    }
}

// RNNoise works on fixed 10 ms frames of 48 kHz audio.
const RNNOISE_FRAME: usize = DenoiseState::FRAME_SIZE;
const RNNOISE_RATE: u32 = 48_000;
// RNNoise expects samples in the 16-bit range (±32768) instead of ±1.0.
const I16_SCALE: f32 = 32768.0;

/// Removes background noise (hiss, fans, keyboards, traffic) while keeping the voice,
/// using RNNoise: a small neural network trained to tell speech from noise.
///
/// RNNoise only accepts whole 480-sample frames, but our blocks can be any size. So the
/// effect always works one frame behind: each incoming sample goes into the frame being
/// filled, and the sample going out comes from the previous, already-cleaned frame.
/// That costs exactly one frame (10 ms) of delay, for any block size.
pub struct NoiseSuppression {
    controls: Arc<EffectControls>,
    // One network per channel: each keeps its own memory of what it has heard.
    states: [Box<DenoiseState<'static>>; 2],
    /// The frame being filled with new input.
    input: [[f32; RNNOISE_FRAME]; 2],
    /// The previous frame, before and after cleaning, being played back.
    dry: [[f32; RNNOISE_FRAME]; 2],
    wet: [[f32; RNNOISE_FRAME]; 2],
    position: usize,
    supported: bool,
}

impl NoiseSuppression {
    pub fn new() -> Self {
        Self {
            controls: Arc::new(EffectControls::new(
                "Noise removal",
                "Removes background noise from your voice (RNNoise). Adds 10 ms of delay.",
                vec![Param::new("Strength", "%", 0.0, 100.0, 100.0)],
            )),
            states: [DenoiseState::new(), DenoiseState::new()],
            input: [[0.0; RNNOISE_FRAME]; 2],
            dry: [[0.0; RNNOISE_FRAME]; 2],
            wet: [[0.0; RNNOISE_FRAME]; 2],
            position: 0,
            supported: true,
        }
    }
}

impl Effect for NoiseSuppression {
    fn controls(&self) -> &Arc<EffectControls> {
        &self.controls
    }

    fn prepare(&mut self, sample_rate: u32) -> Result<(), String> {
        self.supported = sample_rate == RNNOISE_RATE;
        if self.supported {
            Ok(())
        } else {
            Err(format!(
                "Noise removal needs a 48000 Hz mic, but yours runs at {sample_rate} Hz; it will be skipped"
            ))
        }
    }

    fn reset(&mut self) {
        self.input = [[0.0; RNNOISE_FRAME]; 2];
        self.dry = [[0.0; RNNOISE_FRAME]; 2];
        self.wet = [[0.0; RNNOISE_FRAME]; 2];
        self.position = 0;
    }

    fn process(&mut self, block: &mut [Frame]) {
        if !self.supported {
            return;
        }
        // Blends the original back in; both halves are from the same (previous) frame.
        let strength = self.controls.params[0].get() / 100.0;

        for frame in block.iter_mut() {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let dry = self.dry[channel][self.position];
                let wet = self.wet[channel][self.position];
                self.input[channel][self.position] = *sample * I16_SCALE;
                *sample = (dry + (wet - dry) * strength) / I16_SCALE;
            }

            self.position += 1;
            if self.position == RNNOISE_FRAME {
                self.position = 0;
                for channel in 0..2 {
                    // The FFT inside allocates its plans the first time it runs on this
                    // thread, then reuses them: a one-off, not a per-frame allocation.
                    self.states[channel]
                        .process_frame(&mut self.wet[channel], &self.input[channel]);
                    self.dry[channel] = self.input[channel];
                }
            }
        }
    }
}
