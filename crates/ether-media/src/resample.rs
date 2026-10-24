//! Offline, high-quality sample-rate conversion with rubato (windowed sinc).

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

use crate::{DecodedAudio, MediaError};

/// Input frames per resampler call.
const CHUNK: usize = 1024;

fn err(e: impl std::fmt::Display) -> MediaError {
    MediaError::Resample(e.to_string())
}

/// The windowed-sinc resampler every conversion here uses (sinc 256, oversampling 1024,
/// Blackman-Harris², cubic), taking fixed input chunks of `chunk` frames.
pub fn sinc_resampler(ratio: f64, chunk: usize, n_ch: usize) -> Result<Async<f32>, MediaError> {
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: Some(rubato::calculate_cutoff(
            256,
            WindowFunction::BlackmanHarris2,
        )),
        // rubato's cubic sinc-table lookup is offset by one table step (1/oversampling of
        // an input frame); 1024 keeps that below 0.001 frames (~-70 dB error at 1 kHz).
        oversampling_factor: 1024,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    };
    Async::<f32>::new_sinc(ratio, 1.0, &params, chunk, n_ch, FixedAsync::Input).map_err(err)
}

/// Half the sinc length: how far (in input frames) the resampler's output lags its input.
const SINC_HALF: usize = 128;

/// Alignment for `ratio = p / q` in lowest terms: `(zeros to prepend, outputs to drop)` so
/// that output `k` lands exactly on input time `k / ratio`.
///
/// The sinc resampler has no integer delay to trim: its output `j` sits at input time
/// `(j + 1) / ratio - SINC_HALF` (a fractional offset). Prepending `m * q - SINC_HALF`
/// zeros and dropping the first `m * p - 1` outputs, with `m = ceil(SINC_HALF / q)`, moves
/// output `k` onto input time `k / ratio`. `q <= source rate`, so the padding is at most
/// about one second of silence.
pub fn alignment(p: usize, q: usize) -> (usize, usize) {
    let m = SINC_HALF.div_ceil(q);
    (m * q - SINC_HALF, m * p - 1)
}

/// Input and output buffers (one `Vec` per channel) as large as `r` ever needs.
pub fn resampler_buffers(r: &Async<f32>) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let n_ch = r.nbr_channels();
    (
        vec![vec![0.0; r.input_frames_max()]; n_ch],
        vec![vec![0.0; r.output_frames_max()]; n_ch],
    )
}

/// One resampler call: reads `r.input_frames_next()` frames of `inbuf`, writes `outbuf`.
/// Returns `(input frames used, output frames written)`.
pub fn process_chunk(
    r: &mut Async<f32>,
    inbuf: &[Vec<f32>],
    outbuf: &mut [Vec<f32>],
) -> Result<(usize, usize), MediaError> {
    let n_ch = r.nbr_channels();
    let input = SequentialSliceOfVecs::new(inbuf, n_ch, r.input_frames_next()).map_err(err)?;
    let out_len = outbuf.first().map_or(0, Vec::len);
    let mut output = SequentialSliceOfVecs::new_mut(outbuf, n_ch, out_len).map_err(err)?;
    r.process_into_buffer(&input, &mut output, None)
        .map_err(err)
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
    crate::check_rate(target_rate)?;
    crate::check_rate(audio.sample_rate)?;
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
    let mut resampler = sinc_resampler(ratio, CHUNK, n_ch)?;

    let g = gcd(target_rate, audio.sample_rate);
    let (p, q) = ((target_rate / g) as usize, (audio.sample_rate / g) as usize);
    let (pad, drop) = alignment(p, q);
    let total_in = pad + frames;

    let mut out: Vec<Vec<f32>> = (0..n_ch)
        .map(|_| Vec::with_capacity(drop + expected + resampler.output_frames_max()))
        .collect();
    let (mut inbuf, mut outbuf) = resampler_buffers(&resampler);

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
        let (_, n) = process_chunk(&mut resampler, &inbuf, &mut outbuf)?;
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
