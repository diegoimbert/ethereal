//! A dev pack built in code (synthesized audio, signed with [`manifest::DEV_SEED`]) for tests
//! (`scripts/samples/dev-pack.mjs` builds the same shape for the e2e tests). Never shipped as
//! content: the owner selects the real samples (#519).

use std::f64::consts::TAU;

use ether_core::protocol::samples::{SampleCategory, SampleLicense};

use super::manifest::{
    self, DEV_SEED, Envelope, Instrument, Kit, KitPad, Manifest, PackFile, SCHEMA, Zone,
};
use crate::audio_to_midi::kit;

/// A built pack: the envelope served at the manifest URL and the files by relative path.
pub struct DevPack {
    pub manifest: Manifest,
    pub envelope: Vec<u8>,
    pub files: Vec<(String, Vec<u8>)>,
}

fn sine(freq: f64, secs: f64) -> Vec<f32> {
    let n = (secs * kit::RATE as f64) as usize;
    (0..n)
        .map(|k| {
            let t = k as f64 / kit::RATE as f64;
            ((TAU * freq * t).sin() * (-t / (secs / 3.0)).exp() * 0.7) as f32
        })
        .collect()
}

/// Two bars of kicks at 120 bpm.
fn beat() -> Vec<f32> {
    let kick = kit::synth(0);
    let n = 4 * kit::RATE as usize;
    let mut x = vec![0.0f32; n];
    for beat in 0..8 {
        let at = beat * kit::RATE as usize / 2;
        for (k, v) in kick.iter().enumerate().take(n - at) {
            x[at + k] += v * 0.8;
        }
    }
    x
}

fn file(id: &str, name: &str, category: SampleCategory, audio: &[f32]) -> (PackFile, Vec<u8>) {
    let bytes = kit::wav(audio).expect("wav");
    let sha256 = manifest::sha256_hex(&bytes);
    let f = PackFile {
        id: id.into(),
        name: name.into(),
        path: format!("files/{sha256}.wav"),
        sha256,
        size: bytes.len() as u64,
        sample_rate: kit::RATE,
        channels: 1,
        frames: audio.len() as u64,
        category,
        license: SampleLicense::Own,
        tags: vec![],
        bpm: None,
        key: None,
        preview: None,
    };
    (f, bytes)
}

/// Sign `m` into an envelope with the dev key.
pub fn envelope(m: &Manifest) -> Vec<u8> {
    let text = serde_json::to_string(m).expect("manifest");
    let signature = manifest::sign(&text, &DEV_SEED);
    serde_json::to_vec(&Envelope {
        manifest: text,
        signature,
    })
    .expect("envelope")
}

/// The dev pack at `version`. `tone` shifts the instrument's pitch, so two versions can
/// differ in one file.
pub fn dev_pack(version: u64, tone: f64) -> DevPack {
    let mut loop_file = file("loops/beat-120", "beat 120", SampleCategory::Loop, &beat());
    loop_file.0.bpm = Some(120.0);
    loop_file.0.tags = vec!["drums".into()];
    let mut keys = file(
        "keys/sine-c4",
        "sine c4",
        SampleCategory::Instrument,
        &sine(261.63 * tone, 1.0),
    );
    keys.0.key = Some("c major".into());
    let parts = vec![
        file("drums/a/kick", "kick", SampleCategory::Drum, &kit::synth(0)),
        file(
            "drums/a/snare",
            "snare",
            SampleCategory::Drum,
            &kit::synth(1),
        ),
        file("drums/a/hat", "hat", SampleCategory::Drum, &kit::synth(2)),
        loop_file,
        keys,
    ];
    let pad = |note, sample: &str| KitPad {
        note,
        sample: sample.into(),
        name: None,
        choke: None,
    };
    let m = Manifest {
        schema: SCHEMA,
        id: "default".into(),
        version,
        name: "dev pack".into(),
        generated_at: "2026-10-15T00:00:00Z".into(),
        files: parts.iter().map(|(f, _)| f.clone()).collect(),
        kits: vec![Kit {
            id: "a".into(),
            name: "dev kit".into(),
            tags: vec![],
            pads: vec![
                pad(36, "drums/a/kick"),
                pad(38, "drums/a/snare"),
                pad(42, "drums/a/hat"),
            ],
        }],
        instruments: vec![Instrument {
            id: "sine".into(),
            name: "sine".into(),
            tags: vec![],
            zones: vec![Zone {
                sample: "keys/sine-c4".into(),
                root: 60,
                lo_key: 0,
                hi_key: 127,
                lo_vel: 0,
                hi_vel: 127,
            }],
        }],
    };
    DevPack {
        envelope: envelope(&m),
        files: parts.into_iter().map(|(f, b)| (f.path, b)).collect(),
        manifest: m,
    }
}
