//! The factory kit of a drums conversion (#440): a kick, a snare and a closed hi-hat,
//! synthesized in code (deterministic, so every site and platform writes the same bytes) as
//! 16-bit mono WAVs in the project's media, one sampler pad each on the detector's drum keys
//! (`kick_key` / `snare_key` / `hihat_key`), so the new track plays back at once.
//!
//! The files are written before the edit (like freeze renders); the edit adds the media, the
//! pads and their one-shot samplers in the conversion's undo step. A file whose bytes are
//! already in the project (an earlier conversion's kit) is shared, as imports do.

use std::f64::consts::TAU;
use std::io::Cursor;

use ether_core::protocol::model::*;
use ether_model::derive_id;

use crate::handlers::media_file_name;
use crate::media::hash::content_hash;
use crate::store::ProjectStore;

/// Sample rate of the kit's files.
pub(crate) const RATE: u32 = 48_000;

/// One pad of the kit.
#[derive(Clone, Debug)]
pub(crate) struct KitPad {
    pub pad: DrumPadId,
    pub device: DeviceId,
    pub note: u8,
    pub media: MediaRef,
}

/// The kit's sounds, in drum class order (kick, snare, hi-hat).
const SOUNDS: [&str; 3] = ["Kick", "Snare", "Hi-Hat"];

/// Deterministic white noise in -1..=1 (xorshift32).
fn noise(seed: u32) -> impl FnMut() -> f64 {
    let mut s = seed;
    move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s as f64 / u32::MAX as f64 * 2.0 - 1.0
    }
}

/// An RBJ biquad (band-pass or high-pass), direct form I.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn new(high_pass: bool, f0: f64, q: f64) -> Self {
        let w = TAU * f0 / RATE as f64;
        let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
        let a0 = 1.0 + alpha;
        let b = if high_pass {
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0]
        } else {
            [alpha, 0.0, -alpha]
        };
        Self {
            b: b.map(|v| v / a0),
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// Mono samples of sound `i` of [`SOUNDS`].
pub(crate) fn synth(i: usize) -> Vec<f32> {
    let r = RATE as f64;
    let secs = [0.6, 0.35, 0.12][i];
    let n = (secs * r) as usize;
    let mut out = vec![0.0f64; n];
    match i {
        // Kick: a sine sweeping 120 → 50 Hz.
        0 => {
            let mut phase = 0.0;
            for (k, v) in out.iter_mut().enumerate() {
                let t = k as f64 / r;
                let f = 50.0 + 70.0 * (-t / 0.02).exp();
                phase += TAU * f / r;
                *v = phase.sin() * (-t / 0.22).exp();
            }
        }
        // Snare: two decaying tones and band-passed noise (the 1-5 kHz rattle).
        1 => {
            let mut white = noise(0x5eed_0001);
            let mut bp = Biquad::new(false, 2_400.0, 0.6);
            for (k, v) in out.iter_mut().enumerate() {
                let t = k as f64 / r;
                let tone = ((TAU * 185.0 * t).sin() + 0.6 * (TAU * 330.0 * t).sin())
                    * (-t / 0.05).exp()
                    * 0.4;
                *v = tone + 1.2 * bp.run(white()) * (-t / 0.11).exp();
            }
        }
        // Closed hi-hat: noise and metallic partials, high-passed twice at 8 kHz, short.
        _ => {
            let mut white = noise(0x5eed_0002);
            let partials = [8_210.0, 9_130.0, 10_350.0, 11_760.0];
            let (mut hp1, mut hp2) = (
                Biquad::new(true, 8_000.0, 0.7),
                Biquad::new(true, 8_000.0, 0.7),
            );
            for (k, v) in out.iter_mut().enumerate() {
                let t = k as f64 / r;
                let metal: f64 = partials.iter().map(|f| (TAU * f * t).sin()).sum::<f64>()
                    / partials.len() as f64;
                let x = 0.4 * metal + white();
                *v = hp2.run(hp1.run(x)) * (-t / 0.03).exp();
            }
        }
    }
    // Short fades so neither end clicks.
    let fade_in = (0.0005 * r) as usize;
    let fade_out = (0.01 * r) as usize;
    for (k, v) in out.iter_mut().take(fade_in).enumerate() {
        *v *= k as f64 / fade_in as f64;
    }
    for (k, v) in out.iter_mut().rev().take(fade_out).enumerate() {
        *v *= k as f64 / fade_out as f64;
    }
    let peak = out.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-9);
    out.iter().map(|v| (v / peak * 0.89) as f32).collect()
}

/// 16-bit mono WAV bytes of `x`.
pub(crate) fn wav(x: &[f32]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::with_capacity(44 + x.len() * 2));
    let mut w = hound::WavWriter::new(&mut buf, spec).map_err(|e| e.to_string())?;
    for s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * 32_767.0).round() as i16)
            .map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

/// Write the kit's files for a drums conversion whose rack is `rack` and return its pads
/// (ids derived from `rack`). `keys` = (kick, snare, hi-hat); a key repeated by an earlier
/// sound gets no pad of its own.
pub(crate) fn write<S: ProjectStore>(
    store: &mut S,
    project: &Project,
    rack: DeviceId,
    keys: [u8; 3],
) -> Result<Vec<KitPad>, String> {
    let mut pads: Vec<KitPad> = Vec::new();
    for (i, name) in SOUNDS.iter().enumerate() {
        let note = keys[i];
        if pads.iter().any(|p| p.note == note) {
            continue;
        }
        let samples = synth(i);
        let bytes = wav(&samples)?;
        let hash = content_hash(&bytes);
        let id: MediaId = derive_id(rack, 200 + i as u32);
        let file_name = format!("{name}.wav");
        let shared = project
            .media
            .values()
            .filter(|m| m.hash.as_deref() == Some(hash.as_str()))
            .map(|m| m.file.clone())
            .find(|f| store.read(project.id, f).is_ok_and(|b| b == bytes));
        let file = match shared {
            Some(f) => f,
            None => {
                let f = media_file_name(id, &file_name);
                store
                    .write(project.id, &f, &bytes)
                    .map_err(|e| format!("could not write \"{f}\": {e}"))?;
                f
            }
        };
        pads.push(KitPad {
            pad: derive_id(rack, i as u32),
            device: derive_id(rack, 100 + i as u32),
            note,
            media: MediaRef {
                location: Default::default(),
                id,
                name: file_name,
                file,
                sample_rate: RATE,
                channels: 1,
                frames: samples.len() as u64,
                hash: Some(hash),
            },
        });
    }
    Ok(pads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_deterministic_bounded_and_audible() {
        for i in 0..SOUNDS.len() {
            let x = synth(i);
            assert_eq!(x, synth(i));
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!((0.8..=0.9).contains(&peak), "{i}: {peak}");
            assert!(x.last().unwrap().abs() < 1e-3);
            assert_eq!(wav(&x).unwrap(), wav(&x).unwrap());
        }
    }

    /// The converter's own drum detector hears a beat played on the kit as the kit's
    /// classes: converting it lands every hit on its own pad.
    #[test]
    fn the_detector_hears_each_sound_as_its_pad() {
        use ether_media::to_midi::{Detector, Mode, Options};
        let o = Options::default();
        let keys = [o.kick_key, o.snare_key, o.hihat_key];
        // Kick, hat, snare, hat: two bars of eighths at 120 bpm.
        let pattern = [0, 2, 1, 2, 0, 2, 1, 2, 0, 2, 1, 2, 0, 2, 1, 2];
        let sounds: Vec<Vec<f32>> = (0..SOUNDS.len()).map(synth).collect();
        let mut x = vec![0.0f32; (4.6 * RATE as f64) as usize];
        let mut expected = Vec::new();
        for (h, &i) in pattern.iter().enumerate() {
            let at = ((0.2 + 0.25 * h as f64) * RATE as f64) as usize;
            for (k, v) in sounds[i].iter().enumerate() {
                x[at + k] += 0.6 * v;
            }
            expected.push(keys[i]);
        }
        let audio = std::sync::Arc::new(ether_media::DecodedAudio {
            sample_rate: RATE,
            channels: vec![x],
        });
        let mut d = Detector::new(audio, Mode::Drums, o);
        while d.step(4096) < 1.0 {}
        let got: Vec<u8> = d.notes().iter().map(|n| n.pitch).collect();
        assert_eq!(got, expected);
    }
}
