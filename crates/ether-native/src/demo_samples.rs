//! A small set of synthesized demo samples (kick, snare, hat, a bass loop and a chord
//! loop at 120 BPM), written once into a library folder so a fresh install has something
//! to drag from the sample browser. Pure Rust, deterministic, no assets in the repo.

use std::f32::consts::TAU;
use std::io;
use std::path::Path;

const SR: u32 = 44_100;

/// Write the demo samples into `dir` (creating it). Existing files are left alone.
pub fn ensure(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    type Make = fn() -> Vec<f32>;
    let files: [(&str, Make); 5] = [
        ("Kick.wav", kick),
        ("Snare.wav", snare),
        ("Hat.wav", hat),
        ("Bass Loop 120.wav", bass_loop),
        ("Chords Loop 120.wav", chord_loop),
    ];
    for (name, make) in files {
        let path = dir.join(name);
        if !path.exists() {
            crate::store::atomic_write(&path, &wav_mono(&make()))
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
    }
    Ok(())
}

fn secs(s: f32) -> usize {
    (s * SR as f32) as usize
}

fn kick() -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..secs(0.45))
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 45.0 + 110.0 * (-t * 30.0).exp();
            phase += TAU * f / SR as f32;
            0.9 * phase.sin() * (-t * 7.0).exp()
        })
        .collect()
}

/// Deterministic white noise in -1..1.
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

fn snare() -> Vec<f32> {
    let mut n = Noise(0x1234_5678);
    (0..secs(0.3))
        .map(|i| {
            let t = i as f32 / SR as f32;
            let body = (TAU * 190.0 * t).sin() * (-t * 25.0).exp();
            0.5 * body + 0.45 * n.next() * (-t * 14.0).exp()
        })
        .collect()
}

fn hat() -> Vec<f32> {
    let mut n = Noise(0x9e37_79b9);
    let mut prev = 0.0;
    (0..secs(0.08))
        .map(|i| {
            let t = i as f32 / SR as f32;
            let x = n.next();
            let hp = x - prev; // crude high-pass
            prev = x;
            0.35 * hp * (-t * 60.0).exp()
        })
        .collect()
}

fn midi_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Two bars at 120 BPM (4 s): eighth-note bass line, A minor.
fn bass_loop() -> Vec<f32> {
    let notes = [45.0, 45.0, 57.0, 45.0, 48.0, 48.0, 55.0, 43.0];
    let step = secs(0.25);
    let mut out = vec![0.0; step * 16];
    for (k, chunk) in out.chunks_mut(step).enumerate() {
        let f = midi_hz(notes[k % notes.len()]);
        for (i, s) in chunk.iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            let saw = 2.0 * (f * t).fract() - 1.0;
            *s = 0.35 * (0.6 * saw + 0.4 * (TAU * f * t).sin()) * (-t * 6.0).exp();
        }
    }
    out
}

/// Two bars at 120 BPM (4 s): Am – F chords, soft pad.
fn chord_loop() -> Vec<f32> {
    let chords = [[57.0, 60.0, 64.0], [53.0, 57.0, 60.0]];
    let bar = secs(2.0);
    let mut out = vec![0.0; bar * 2];
    for (k, chunk) in out.chunks_mut(bar).enumerate() {
        for (i, s) in chunk.iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            let env = (t * 20.0).min(1.0) * (1.0 - t / 2.0).max(0.0);
            let v: f32 = chords[k]
                .iter()
                .map(|n| (TAU * midi_hz(*n) * t).sin() + 0.3 * (TAU * 2.0 * midi_hz(*n) * t).sin())
                .sum();
            *s = 0.12 * v * env;
        }
    }
    out
}

/// 16-bit PCM mono WAV.
fn wav_mono(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_decodable_wavs_once() {
        let tmp = crate::test_util::TempDir::new("demo-samples");
        super::ensure(tmp.path()).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 5, "{names:?}");
        for n in &names {
            let bytes = std::fs::read(tmp.path().join(n)).unwrap();
            let audio = ether_media::decode(&bytes, Some("wav")).unwrap();
            assert!(audio.frames() > 1000, "{n}");
            let peak = audio.channels[0].iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05 && peak <= 1.0, "{n}: {peak}");
        }
        // Idempotent.
        super::ensure(tmp.path()).unwrap();
    }
}
