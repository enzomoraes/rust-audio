use cpal::traits::{DeviceTrait, HostTrait};

// ~10 ms per callback: low latency without waking the audio thread too often.
const TARGET_CALLBACK_MS: u32 = 10;
// Used when the backend can't report its buffer size range before the stream starts.
const FALLBACK_CALLBACK_FRAMES: usize = 2048;

pub struct DeviceConfig {
    pub stream: cpal::StreamConfig,
    pub callback_frames: usize,
}

pub fn default_mic() -> Result<cpal::Device, String> {
    cpal::default_host()
        .default_input_device()
        .ok_or_else(|| "no default microphone found".to_string())
}

pub fn default_speaker() -> Result<cpal::Device, String> {
    cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no default speaker found".to_string())
}

pub fn name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "unknown device".to_string())
}

pub fn input_config(device: &cpal::Device) -> Result<DeviceConfig, String> {
    device
        .default_input_config()
        .map(|supported| with_fixed_buffer(&supported))
        .map_err(|err| format!("could not read the mic config: {err}"))
}

pub fn output_config(device: &cpal::Device) -> Result<DeviceConfig, String> {
    device
        .default_output_config()
        .map(|supported| with_fixed_buffer(&supported))
        .map_err(|err| format!("could not read the speaker config: {err}"))
}

fn with_fixed_buffer(supported: &cpal::SupportedStreamConfig) -> DeviceConfig {
    let mut stream = supported.config();
    let callback_frames = match *supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            // 10 ms: 480 frames at 48 kHz, 441 at 44.1 kHz, 960 at 96 kHz.
            let frames = (stream.sample_rate * TARGET_CALLBACK_MS / 1000).clamp(min, max);
            stream.buffer_size = cpal::BufferSize::Fixed(frames);
            frames as usize
        }
        cpal::SupportedBufferSize::Unknown => FALLBACK_CALLBACK_FRAMES,
    };
    DeviceConfig {
        stream,
        callback_frames,
    }
}
