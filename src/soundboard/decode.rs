use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::dsp::Frame;

/// Decodes an MP3/WAV file into stereo frames at `target_rate`.
pub fn decode_file(path: &Path, target_rate: u32) -> Result<Vec<Frame>, String> {
    let file = File::open(path).map_err(|err| format!("could not open the file: {err}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|err| format!("unsupported file: {err}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("the file has no audio track")?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or("the file has no audio track")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|err| format!("unsupported codec: {err}"))?;

    let mut frames = Vec::new();
    let mut samples: Vec<f32> = Vec::new();
    let mut source_rate = target_rate;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(err) => return Err(format!("could not read the file: {err}")),
        };
        if packet.track_id != track_id {
            continue;
        }

        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // A corrupt packet loses a few milliseconds; the rest of the file is still fine.
            Err(Error::DecodeError(_)) => continue,
            Err(err) => return Err(format!("could not decode the file: {err}")),
        };
        source_rate = buffer.spec().rate();
        let channels = buffer.spec().channels().count().max(1);

        samples.resize(buffer.samples_interleaved(), 0.0);
        buffer.copy_to_slice_interleaved(&mut samples);
        frames.extend(samples.chunks(channels).map(|frame| match frame {
            [mono] => [*mono, *mono],
            [left, right, ..] => [*left, *right],
            [] => [0.0, 0.0],
        }));
    }

    if frames.is_empty() {
        return Err("the file has no audio".into());
    }
    Ok(resample(&frames, source_rate, target_rate))
}

/// Linear interpolation between neighboring frames. Good enough for sound effects;
/// a real resampler (e.g. the `rubato` crate) would avoid some high-frequency artifacts.
fn resample(frames: &[Frame], from: u32, to: u32) -> Vec<Frame> {
    if from == to {
        return frames.to_vec();
    }
    let step = from as f64 / to as f64;
    let out_len = (frames.len() as f64 / step) as usize;
    let last = frames.len() - 1;

    (0..out_len)
        .map(|index| {
            let position = index as f64 * step;
            let before = (position as usize).min(last);
            let after = (before + 1).min(last);
            let t = (position - before as f64) as f32;
            let [l0, r0] = frames[before];
            let [l1, r1] = frames[after];
            [l0 + (l1 - l0) * t, r0 + (r1 - r0) * t]
        })
        .collect()
}
