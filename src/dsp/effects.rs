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

// Closing a few dB below the opening threshold keeps the gate from chattering
// open/closed when the level hovers right at the threshold.
const GATE_HYSTERESIS_DB: f32 = 6.0;
// Stays open this long after the voice drops, so short pauses between syllables
// don't cut words apart.
const GATE_HOLD_SECONDS: f32 = 0.05;
// Opening fast keeps the start of words; a few ms instead of instant avoids a click.
const GATE_ATTACK_SECONDS: f32 = 0.002;
// How fast the level detector forgets a peak.
const DETECTOR_DECAY_SECONDS: f32 = 0.02;

/// Silences the mic while you're not talking, so background noise doesn't get
/// through between words. While you talk the gate is open and the noise passes
/// with the voice: it only cleans up the pauses.
///
///   level above Threshold  -> open (the voice passes)
///   level below Threshold - 6 dB for longer than the hold -> fades closed over Release
pub struct NoiseGate {
    controls: Arc<EffectControls>,
    sample_rate: f32,
    /// Smoothed peak level of the input, linear.
    envelope: f32,
    /// Current gain applied to the signal: 0 = closed, 1 = open.
    gain: f32,
    open: bool,
    hold_remaining: usize,
}

impl NoiseGate {
    pub fn new(threshold_db: f32) -> Self {
        Self {
            controls: Arc::new(EffectControls::new(
                "Noise gate",
                "Mutes the mic while you're not talking, so noise doesn't leak between words.",
                vec![
                    Param::new("Threshold", " dB", -80.0, 0.0, threshold_db),
                    Param::new("Release", " ms", 10.0, 1000.0, 150.0),
                ],
            )),
            sample_rate: 48_000.0,
            envelope: 0.0,
            gain: 0.0,
            open: false,
            hold_remaining: 0,
        }
    }

    /// How much of the remaining distance a one-pole smoother covers per sample,
    /// to get about 63% of the way there in `seconds`.
    fn coefficient(&self, seconds: f32) -> f32 {
        1.0 - (-1.0 / (seconds * self.sample_rate)).exp()
    }
}

impl Effect for NoiseGate {
    fn controls(&self) -> &Arc<EffectControls> {
        &self.controls
    }

    fn prepare(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate as f32;
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain = 0.0;
        self.open = false;
        self.hold_remaining = 0;
    }

    fn process(&mut self, block: &mut [Frame]) {
        let threshold_db = self.controls.params[0].get();
        let open_level = 10f32.powf(threshold_db / 20.0);
        let close_level = 10f32.powf((threshold_db - GATE_HYSTERESIS_DB) / 20.0);
        let release_seconds = self.controls.params[1].get() / 1000.0;

        let attack = self.coefficient(GATE_ATTACK_SECONDS);
        let release = self.coefficient(release_seconds);
        let detector_decay = 1.0 - self.coefficient(DETECTOR_DECAY_SECONDS);
        let hold_samples = (GATE_HOLD_SECONDS * self.sample_rate) as usize;

        for frame in block.iter_mut() {
            // Both channels share one gain, so the stereo image doesn't shift.
            let peak = frame[0].abs().max(frame[1].abs());
            self.envelope = peak.max(self.envelope * detector_decay);

            if self.envelope >= open_level {
                self.open = true;
                self.hold_remaining = hold_samples;
            } else if self.envelope < close_level {
                if self.hold_remaining > 0 {
                    self.hold_remaining -= 1;
                } else {
                    self.open = false;
                }
            }

            let (target, speed) = if self.open {
                (1.0, attack)
            } else {
                (0.0, release)
            };
            self.gain += (target - self.gain) * speed;

            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
    }
}
