use std::f32::consts::TAU;

pub type Frame = [f32; 2];

pub struct Pipeline {
    effects: Vec<Box<dyn Effect>>,
    data: Vec<Frame>,
}

impl Pipeline {
    pub fn new(block_size: usize) -> Self {
        Pipeline {
            effects: vec![],
            data: vec![[0.0, 0.0]; block_size],
        }
    }

    pub fn add(mut self, effect: impl Effect + 'static) -> Self {
        self.effects.push(Box::new(effect));
        self
    }

    pub fn process(
        &mut self,
        mut input: impl Iterator<Item = Frame>,
        mut output: impl FnMut(&[Frame]),
    ) {
        loop {
            let n = self
                .data
                .iter_mut()
                .zip(input.by_ref())
                .map(|(pipeline_frame, input_frame)| *pipeline_frame = input_frame)
                .count();

            let block: &mut [Frame] = &mut self.data[..n];
            for effect in self.effects.iter_mut() {
                effect.process(block);
            }
            output(block);

            // A partially filled block means the input ran out.
            if n < self.data.len() {
                break;
            }
        }
    }
}
pub trait Effect: Send {
    fn process(&mut self, block: &mut [Frame]);
}

pub struct Gain {
    factor: f32,
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
///   Pipeline::new(block_size)
///       .add(Gain::new(20.0))
///       .add(SoftClipping {})
pub struct SoftClipping;

/// Multiplies the signal by a sine carrier, turning every frequency f into
/// f - carrier and f + carrier. The voice's harmonics stop being integer
/// multiples of one pitch, so it sounds metallic and robotic.
///
///   ~30 Hz: buzzy Dalek voice | 100-300 Hz: metallic, bell-like
pub struct RingModulation {
    phase: f32,
    step: f32,
}

impl RingModulation {
    pub fn new(frequency: f32, sample_rate: u32) -> Self {
        RingModulation {
            phase: 0.0,
            step: TAU * frequency / sample_rate as f32,
        }
    }
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
        Gain {
            factor: 10f32.powf(db / 20.0),
        }
    }
}

impl Effect for Gain {
    fn process(&mut self, block: &mut [Frame]) {
        for frame in block.iter_mut() {
            for sample in frame.iter_mut() {
                *sample *= self.factor;
            }
        }
    }
}

impl Effect for SoftClipping {
    fn process(&mut self, block: &mut [Frame]) {
        for frame in block.iter_mut() {
            for sample in frame.iter_mut() {
                *sample = sample.tanh();
            }
        }
    }
}

impl Effect for RingModulation {
    fn process(&mut self, block: &mut [Frame]) {
        for frame in block.iter_mut() {
            // One carrier value per frame, so L and R stay in sync.
            let carrier = self.phase.sin();
            for sample in frame.iter_mut() {
                *sample *= carrier;
            }

            self.phase += self.step;
            if self.phase >= TAU {
                self.phase -= TAU;
            }
        }
    }
}
