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
/// The output is time-aligned with the input (output frame `k` is input time
/// `k / ratio`, exactly) and has exactly
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
        // rubato's cubic sinc-table lookup is offset by one table step (1/oversampling of
        // an input frame); 1024 keeps that below 0.001 frames (~-70 dB error at 1 kHz).
        oversampling_factor: 1024,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, n_ch).map_err(err)?;

    // Alignment. `SincFixedIn` has no integer delay to trim: its output `j` sits at input
    // time `(j + 1) / ratio - 1` (a fractional offset). With `ratio = p / q` in lowest
    // terms, prepending `q - 1` zeros and dropping the first `p - 1` outputs makes output
    // `k` land exactly on input time `k / ratio`. `q <= source rate`, so the padding is
    // at most one second of silence.
    let g = gcd(target_rate, audio.sample_rate);
    let (p, q) = ((target_rate / g) as usize, (audio.sample_rate / g) as usize);
    let pad = q - 1;
    let drop = p - 1;
    let total_in = pad + frames;

    let mut out: Vec<Vec<f32>> = (0..n_ch)
        .map(|_| Vec::with_capacity(drop + expected + resampler.output_frames_max()))
        .collect();
    let mut inbuf = resampler.input_buffer_allocate(true);
    let mut outbuf = resampler.output_buffer_allocate(true);

    // Virtual input: `pad` zeros, then the audio, then zeros forever (flushes the tail).
    let mut pos = 0;
    while out[0].len() < drop + expected {
        let need = resampler.input_frames_next();
        for (dst, src) in inbuf.iter_mut().zip(&audio.channels) {
            dst.resize(need, 0.0);
            for (i, d) in dst.iter_mut().enumerate() {
                let v = pos + i;
                *d = if v >= pad && v < total_in {
                    src[v - pad]
                } else {
                    0.0
                };
            }
        }
        let (_, n) = resampler
            .process_into_buffer(&inbuf, &mut outbuf, None)
            .map_err(err)?;
        if n == 0 && pos > total_in + 4 * CHUNK {
            break; // cannot happen with SincFixedIn; guard against an endless loop
        }
        for (o, b) in out.iter_mut().zip(&outbuf) {
            o.extend_from_slice(&b[..n]);
        }
        pos += need;
    }

    for ch in &mut out {
        ch.drain(..drop.min(ch.len()));
        ch.resize(expected, 0.0);
    }
    Ok(DecodedAudio {
        sample_rate: target_rate,
        channels: out,
    })
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
