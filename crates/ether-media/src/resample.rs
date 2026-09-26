//! Offline, high-quality sample-rate conversion with rubato (windowed sinc).

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::{DecodedAudio, MediaError};

/// Input frames per resampler call.
const CHUNK: usize = 1024;

fn err(e: impl std::fmt::Display) -> MediaError {
    MediaError::Resample(e.to_string())
}

/// Number of frames `frames` source frames become at `ratio` (target / source rate).
pub fn resampled_len(frames: usize, source_rate: u32, target_rate: u32) -> usize {
    if source_rate == 0 {
        return 0;
    }
    (frames as u128 * target_rate as u128).div_ceil(source_rate as u128) as usize
}

/// High-quality resample to `target_rate` (no-op clone if equal).
///
/// The output is aligned with the input (the filter delay is compensated) and has exactly
/// `ceil(frames * target_rate / sample_rate)` frames.
pub fn resample(audio: &DecodedAudio, target_rate: u32) -> Result<DecodedAudio, MediaError> {
    if target_rate == 0 || audio.sample_rate == 0 {
        return Err(MediaError::Resample("sample rate must be > 0".into()));
    }
    if audio.sample_rate == target_rate {
        return Ok(audio.clone());
    }
    let frames = audio.frames();
    let n_ch = audio.channels.len();
    let expected = resampled_len(frames, audio.sample_rate, target_rate);
    if frames == 0 || n_ch == 0 {
        return Ok(DecodedAudio {
            sample_rate: target_rate,
            channels: vec![Vec::new(); n_ch],
        });
    }

    let ratio = target_rate as f64 / audio.sample_rate as f64;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: rubato::calculate_cutoff(256, WindowFunction::BlackmanHarris2),
        oversampling_factor: 256,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, n_ch).map_err(err)?;
    let delay = resampler.output_delay();
    let mut out: Vec<Vec<f32>> = (0..n_ch)
        .map(|_| Vec::with_capacity(expected + delay + resampler.output_frames_max()))
        .collect();
    let mut buf = resampler.output_buffer_allocate(true);

    let append = |out: &mut Vec<Vec<f32>>, buf: &[Vec<f32>], n: usize| {
        for (o, b) in out.iter_mut().zip(buf) {
            o.extend_from_slice(&b[..n]);
        }
    };

    let mut pos = 0;
    while pos < frames {
        let need = resampler.input_frames_next();
        let end = (pos + need).min(frames);
        let slices: Vec<&[f32]> = audio.channels.iter().map(|c| &c[pos..end]).collect();
        let (_, n) = if end - pos == need {
            resampler.process_into_buffer(&slices, &mut buf, None)
        } else {
            resampler.process_partial_into_buffer(Some(&slices), &mut buf, None)
        }
        .map_err(err)?;
        append(&mut out, &buf, n);
        pos = end;
    }
    // Flush the filter tail.
    while out[0].len() < delay + expected {
        let (_, n) = resampler
            .process_partial_into_buffer(None::<&[&[f32]]>, &mut buf, None)
            .map_err(err)?;
        if n == 0 {
            break;
        }
        append(&mut out, &buf, n);
    }

    for ch in &mut out {
        ch.drain(..delay.min(ch.len()));
        ch.resize(expected, 0.0);
    }
    Ok(DecodedAudio {
        sample_rate: target_rate,
        channels: out,
    })
}
