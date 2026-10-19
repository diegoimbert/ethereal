//! The built-in kit (pack `builtin`): the kick, snare and closed hi-hat that audio-to-MIDI
//! synthesizes (#500, `audio_to_midi::kit`), always available offline, never downloaded.
//! No third-party audio: every byte comes from code, deterministically.

use ether_core::protocol::samples::{
    BUILTIN_PACK, KitInfo, KitPadInfo, SampleCategory, SampleInfo, SampleLicense,
};

use crate::audio_to_midi::kit;

/// (id, display name, General MIDI note, `kit::synth` index).
const SOUNDS: [(&str, &str, u8, usize); 3] = [
    ("kick", "kick", 36, 0),
    ("snare", "snare", 38, 1),
    ("hihat", "closed hat", 42, 2),
];

/// The built-in kit's id (qualified).
pub const KIT: &str = "builtin/basic";

fn sound(id: &str) -> Option<(&'static str, &'static str, u8, usize)> {
    SOUNDS.iter().copied().find(|s| s.0 == id)
}

/// The WAV bytes of built-in sample `id` (unqualified).
pub fn bytes(id: &str) -> Option<Vec<u8>> {
    let (.., i) = sound(id)?;
    kit::wav(&kit::synth(i)).ok()
}

pub fn samples() -> Vec<SampleInfo> {
    SOUNDS
        .iter()
        .map(|&(id, name, _, i)| SampleInfo {
            id: format!("{BUILTIN_PACK}/{id}"),
            name: name.to_string(),
            category: SampleCategory::Drum,
            tags: vec!["built-in".into()],
            bpm: None,
            key: None,
            license: SampleLicense::Own,
            size: 0,
            sample_rate: kit::RATE,
            channels: 1,
            frames: kit::synth(i).len() as u64,
            cached: true,
        })
        .collect()
}

pub fn kit_info() -> KitInfo {
    KitInfo {
        id: KIT.to_string(),
        name: "basic kit".into(),
        tags: vec!["built-in".into()],
        pads: SOUNDS
            .iter()
            .map(|&(id, name, note, _)| KitPadInfo {
                note,
                sample: format!("{BUILTIN_PACK}/{id}"),
                name: name.to_string(),
                choke: None,
            })
            .collect(),
    }
}
