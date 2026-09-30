use std::f32::consts::TAU;
use std::sync::Arc;

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

    fn prepare(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate as f32;
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
