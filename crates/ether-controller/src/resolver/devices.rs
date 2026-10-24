//! Missing devices (owner request "missing devices"; `DeviceKind::Missing` placeholders for
//! devices Ethereal could not create): built-in replacements suggested by what the device
//! was (pure, unit-tested). Replacing goes through [`super::replace`] like a plugin swap.

use ether_core::protocol::model::{BuiltinDeviceType as T, MissingDevice, MissingRole};
use ether_core::protocol::resolver::{DeviceCandidate, Replacement};

/// Words of a device's description → the built-in doing the same kind of thing.
const KEYWORDS: &[(&[&str], T, &str)] = &[
    (
        &["reverb", "hybrid", "verb", "convolution"],
        T::Reverb,
        "reverb",
    ),
    (&["grain"], T::GrainDelay, "grain delay"),
    (
        &["shifter", "ringmod", "ring", "frequency", "freqshift"],
        T::Shifter,
        "shifter",
    ),
    (&["echo", "delay"], T::Delay, "delay"),
    (&["chorus", "ensemble"], T::Chorus, "chorus"),
    (&["phaser"], T::Phaser, "phaser"),
    (&["flanger"], T::Flanger, "flanger"),
    (&["tremolo"], T::Tremolo, "tremolo"),
    (
        &["vibrato", "wow", "flutter", "wobble"],
        T::Vibrato,
        "vibrato",
    ),
    (
        &[
            "pump",
            "pumper",
            "duck",
            "ducker",
            "sidechain",
            "trancegate",
        ],
        T::VolumeLfo,
        "volume lfo",
    ),
    (&["filter", "wah"], T::AutoFilter, "filter"),
    (&["pedal", "overdrive", "fuzz"], T::Pedal, "pedal"),
    (
        &[
            "distortion",
            "drive",
            "tube",
            "amp",
            "saturator",
            "roar",
            "buss",
        ],
        T::Saturator,
        "distortion",
    ),
    (&["erosion"], T::Erosion, "erosion"),
    (&["redux", "crush", "vinyl", "lofi"], T::Bitcrusher, "lo-fi"),
    (&["multiband"], T::MultibandCompressor, "multiband dynamics"),
    (
        &["compressor", "glue", "dynamics"],
        T::Compressor,
        "compressor",
    ),
    (&["gate"], T::Gate, "gate"),
    (
        &["maximizer", "maximiser", "brickwall"],
        T::Maximizer,
        "maximizer",
    ),
    (&["limiter"], T::Limiter, "limiter"),
    (&["eq", "equalizer"], T::Eq, "EQ"),
    (&["utility", "gain", "width"], T::Utility, "utility"),
    (&["transient"], T::TransientShaper, "transient shaper"),
    (&["shaper", "volumeshaper"], T::Shaper, "shaper"),
    (&["loudness", "lufs"], T::Loudness, "loudness meter"),
    (&["spectrum", "analyzer"], T::SpectrumAnalyzer, "analyzer"),
    (&["tuner"], T::Tuner, "tuner"),
    (&["arpeggiator", "arp"], T::Arpeggiator, "arpeggiator"),
    (&["chord"], T::Chord, "chord"),
    (&["scale"], T::ScaleQuantize, "scale"),
    (&["velocity"], T::Velocity, "velocity"),
    (&["random"], T::Randomizer, "random"),
    (&["pitch", "shifter"], T::Pitch, "pitch"),
    (&["sampler", "simpler", "sample"], T::Sampler, "sampler"),
    (
        &[
            "drift",
            "analog",
            "operator",
            "wavetable",
            "synth",
            "meld",
            "poli",
        ],
        T::PolySynth,
        "synth",
    ),
];

fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    // Split camel case and non-letters ("GrainDelay" → grain, delay).
    let mut prev_lower = false;
    for c in s.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur).to_lowercase());
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
        prev_lower = c.is_lowercase();
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur.to_lowercase());
    }
    out
}

fn fits(role: MissingRole, ty: T) -> bool {
    use ether_core::protocol::devices::DeviceCategory as C;
    let c = ether_devices::descriptor(ty).category;
    matches!(
        (role, c),
        (MissingRole::Instrument, C::Instrument)
            | (MissingRole::AudioEffect, C::AudioEffect)
            | (MissingRole::NoteEffect, C::NoteEffect)
    )
}

/// Built-in replacements for a missing device, best first: the kind of device its name says
/// (0.6), else the role's generic choice (0.3).
pub fn candidates(m: &MissingDevice) -> Vec<DeviceCandidate> {
    let ws: Vec<String> = words(&m.description)
        .into_iter()
        .chain(words(m.source_id.rsplit(':').next().unwrap_or("")))
        .filter(|w| w != "live" && w != "s")
        .collect();
    let mut out: Vec<DeviceCandidate> = Vec::new();
    let mut push = |ty: T, confidence: f32, reason: String| {
        let already = out
            .iter()
            .any(|c| matches!(c.replacement, Replacement::Builtin { device } if device == ty));
        if !already && fits(m.role, ty) {
            out.push(DeviceCandidate {
                replacement: Replacement::Builtin { device: ty },
                name: ether_devices::descriptor(ty).name,
                confidence,
                reason,
            });
        }
    };
    for (keys, ty, what) in KEYWORDS {
        if ws.iter().any(|w| keys.contains(&w.as_str())) {
            push(*ty, 0.6, format!("same kind of device: {what}"));
        }
    }
    let fallback = match m.role {
        MissingRole::Instrument => T::PolySynth,
        MissingRole::AudioEffect => T::Utility,
        MissingRole::NoteEffect => T::Velocity,
    };
    push(
        fallback,
        0.3,
        "A built-in device of the same role (its parameters may not match)".into(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(description: &str, source_id: &str, role: MissingRole) -> MissingDevice {
        MissingDevice {
            description: description.into(),
            source_id: source_id.into(),
            origin: String::new(),
            role,
            params: Vec::new(),
        }
    }

    fn first(m: &MissingDevice) -> T {
        match candidates(m)[0].replacement {
            Replacement::Builtin { device } => device,
            _ => unreachable!(),
        }
    }

    #[test]
    fn suggests_by_kind_then_role() {
        let reverb = missing(
            "Live's Hybrid Reverb",
            "ableton:Hybrid",
            MissingRole::AudioEffect,
        );
        assert_eq!(first(&reverb), T::Reverb);
        assert!((candidates(&reverb)[0].confidence - 0.6).abs() < 1e-6);
        let grain = missing(
            "Live's Grain Delay",
            "ableton:GrainDelay",
            MissingRole::AudioEffect,
        );
        assert_eq!(first(&grain), T::GrainDelay);
        let pedal = missing("Live's Pedal", "ableton:Pedal", MissingRole::AudioEffect);
        assert_eq!(first(&pedal), T::Pedal);
        let shifter = missing(
            "Live's Shifter",
            "ableton:Shifter",
            MissingRole::AudioEffect,
        );
        assert_eq!(first(&shifter), T::Shifter);
        let op = missing(
            "Live's Operator",
            "ableton:Operator",
            MissingRole::Instrument,
        );
        assert_eq!(first(&op), T::PolySynth);
        // Nothing known: the role's generic choice, low confidence.
        let odd = missing("Bitwig Polymer", "Bitwig Polymer", MissingRole::Instrument);
        assert_eq!(first(&odd), T::PolySynth);
        let fx = missing("Mystery", "x", MissingRole::AudioEffect);
        let c = candidates(&fx);
        assert_eq!(c.len(), 1);
        assert!(c[0].confidence < 0.5);
    }

    #[test]
    fn never_suggests_another_role() {
        // "Sampler" words on an audio effect: no instrument suggested.
        let m = missing("Sample Shifter", "x", MissingRole::AudioEffect);
        assert!(candidates(&m).iter().all(|c| match c.replacement {
            Replacement::Builtin { device } => fits(MissingRole::AudioEffect, device),
            _ => false,
        }));
    }
}
