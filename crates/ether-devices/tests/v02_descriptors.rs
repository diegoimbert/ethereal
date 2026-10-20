//! v0.2 device contracts: descriptor ↔ mock parity, placeholder behaviour, RT
//! safety and factory presets. Device nodes keep these passing (update the mock JSON with
//! `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v02_descriptors`, never
//! by hand) and add their own behaviour tests next to them.

use std::collections::BTreeMap;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor};
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, PresetDevice, PresetModTarget, PresetRack, load_preset,
};
use ether_core::{
    AudioBuffers, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Mock descriptor file per group (`ui/src/transport/mock/devices/<file>.json`), owned by
/// the group's node.
const GROUPS: &[(&str, &[BuiltinDeviceType])] = &[
    ("polySynth", &[BuiltinDeviceType::PolySynth]),
    ("multisampler", &[BuiltinDeviceType::MultiSampler]),
    (
        "fxColor",
        &[
            BuiltinDeviceType::Saturator,
            BuiltinDeviceType::Bitcrusher,
            BuiltinDeviceType::AutoFilter,
        ],
    ),
    (
        "fxModulation",
        &[
            BuiltinDeviceType::Chorus,
            BuiltinDeviceType::Phaser,
            BuiltinDeviceType::Flanger,
            BuiltinDeviceType::Tremolo,
            BuiltinDeviceType::Vibrato,
            BuiltinDeviceType::VolumeLfo,
        ],
    ),
    (
        "fxDynamics",
        &[
            BuiltinDeviceType::Gate,
            BuiltinDeviceType::MultibandCompressor,
            BuiltinDeviceType::TransientShaper,
        ],
    ),
    (
        "fxAnalysis",
        &[
            BuiltinDeviceType::SpectrumAnalyzer,
            BuiltinDeviceType::Tuner,
        ],
    ),
    (
        "midiFx",
        &[
            BuiltinDeviceType::Arpeggiator,
            BuiltinDeviceType::Chord,
            BuiltinDeviceType::ScaleQuantize,
            BuiltinDeviceType::NoteLength,
            BuiltinDeviceType::Velocity,
            BuiltinDeviceType::Randomizer,
            BuiltinDeviceType::Pitch,
        ],
    ),
    (
        "racks",
        &[
            BuiltinDeviceType::InstrumentRack,
            BuiltinDeviceType::AudioEffectRack,
            BuiltinDeviceType::MidiEffectRack,
        ],
    ),
];

fn mock_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/transport/mock/devices")
}

fn check_file(name: &str, expected: &serde_json::Value) {
    let path = mock_dir().join(format!("{name}.json"));
    let text = format!("{}\n", serde_json::to_string_pretty(expected).unwrap());
    if std::env::var_os("UPDATE_MOCK_DESCRIPTORS").is_some() {
        std::fs::create_dir_all(mock_dir()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let found = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (run with UPDATE_MOCK_DESCRIPTORS=1)",
            path.display()
        )
    });
    let found: serde_json::Value = serde_json::from_str(&found).unwrap();
    assert_eq!(
        &found,
        expected,
        "{} is stale: run `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v02_descriptors`",
        path.display()
    );
}

#[test]
fn mock_descriptors_match_rust() {
    let mut covered = 0;
    for (file, types) in GROUPS {
        let map: BTreeMap<String, DeviceDescriptor> = types
            .iter()
            .map(|t| (format!("{t:?}"), ether_devices::descriptor(*t)))
            .collect();
        covered += types.len();
        check_file(file, &serde_json::to_value(map).unwrap());
    }
    // Every v0.2 type belongs to exactly one group (the 9 earlier ones are in
    // `builtinDevices.ts`, the 3 v0.3 ones in `v03_descriptors.rs`, the FM Synth in
    // `fm_synth.rs`; the later Vibrato and Volume LFO are in the fxModulation group).
    assert_eq!(covered, BuiltinDeviceType::ALL.len() - 9 - 3 - 1);
    // The EQ carries the v0.2 `EqCurve` layout: generated too.
    check_file(
        "eq",
        &serde_json::to_value(ether_devices::descriptor(BuiltinDeviceType::Eq)).unwrap(),
    );
    check_file(
        "modulators",
        &serde_json::to_value(ether_devices::modulators::all()).unwrap(),
    );
}

fn prepared(node: &mut dyn Node) {
    node.prepare(&PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: 256,
        max_events_per_block: 64,
    });
}

#[test]
fn placeholders_follow_their_category_without_allocating() {
    for (_, types) in GROUPS {
        for &t in *types {
            let desc = ether_devices::descriptor(t);
            let mut node = ether_devices::create(&BuiltinDevice::new(t), &NoSamples);
            prepared(node.as_mut());
            let (n_in, n_out) = node.channels();
            let input = [vec![0.5f32; 256], vec![-0.25f32; 256]];
            let mut out = [vec![0.0f32; 256], vec![0.0f32; 256]];
            let mut out_events = EventBuffer::with_capacity(64);
            let events = [
                ProcessEvent {
                    offset: 0,
                    kind: EventKind::Param {
                        param: desc.params[0].id,
                        value: desc.params[0].max,
                    },
                },
                ProcessEvent {
                    offset: 3,
                    kind: EventKind::NoteOn {
                        note_id: 1,
                        channel: 0,
                        key: 60,
                        velocity: 0.8,
                    },
                },
            ];
            let transport = TransportInfo {
                playing: true,
                ..TransportInfo::STOPPED
            };
            {
                let ins: [&[f32]; 2] = [&input[0], &input[1]];
                let (l, r) = out.split_at_mut(1);
                let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
                let mut ctx = ProcessContext {
                    sample_rate: 48_000.0,
                    frames: 256,
                    transport: &transport,
                    events: &events,
                    out_events: &mut out_events,
                };
                let mut buffers = AudioBuffers {
                    inputs: &ins[..n_in as usize],
                    outputs: &mut outs[..n_out as usize],
                };
                assert_no_alloc(|| {
                    node.process(&mut ctx, &mut buffers);
                });
            }
            assert_eq!(
                node.param(desc.params[0].id),
                Some(desc.params[0].max),
                "{t:?}"
            );
            // Placeholder category behaviour (pass-through, silence, MIDI-thru): only for the
            // types the registry still lists as unimplemented (`ether_devices::UNIMPLEMENTED`).
            if ether_devices::implemented(t) {
                continue;
            }
            match desc.category {
                DeviceCategory::NoteEffect => {
                    assert_eq!(
                        (n_in, n_out),
                        (0, 0),
                        "{t:?}: MIDI effects leave audio alone"
                    );
                    assert_eq!((desc.audio_inputs, desc.audio_outputs), (0, 0), "{t:?}");
                    // MIDI-thru: the note goes on, the param does not.
                    assert_eq!(out_events.as_slice(), &events[1..], "{t:?}");
                }
                DeviceCategory::AudioEffect => {
                    assert_eq!(out[0][10], 0.5, "{t:?}: pass-through");
                    assert!(out_events.is_empty());
                }
                DeviceCategory::Instrument => {
                    assert!(desc.midi_input, "{t:?}");
                }
            }
        }
    }
}

/// Every integer or stepped param of every built-in (and modulator) carries a plain step:
/// enums/toggles (labels), semitone transposes (not fine tunes), keys, voices and counts.
#[test]
fn integer_params_have_a_step() {
    use ether_core::protocol::devices::{ParamInfo, ParamUnit};
    const INTEGER_NAMES: [&str; 11] = [
        "Transpose",
        "Root Key",
        "Voices",
        // "Unison Voices" is matched by "Voices"; "Unison Detune" / "Unison Blend" are not
        // integers.
        "Unison (legacy)",
        "Octave",
        "Octaves",
        "Steps",
        "Seed",
        "Semitones",
        "Choke",
        "Bend Range",
    ];
    let check = |owner: &str, p: &ParamInfo| {
        let integer = p.labels.is_some()
            || (p.unit == ParamUnit::Semitones && !p.name.contains("Fine"))
            || INTEGER_NAMES.iter().any(|n| p.name.contains(n));
        if integer {
            assert_eq!(
                p.step,
                Some(1.0),
                "{owner}: param {:?} ({}) needs step 1",
                p.id,
                p.name
            );
        }
    };
    for t in BuiltinDeviceType::ALL {
        for p in &ether_devices::descriptor(t).params {
            check(&format!("{t:?}"), p);
        }
    }
    for m in ether_devices::modulators::all() {
        for p in &m.params {
            check(&m.name, p);
        }
    }
}

/// v0.2 devices with a detector take a sidechain (stereo), keyed through
/// `Node::process_sidechain` like the compressor and limiter.
#[test]
fn detector_devices_take_a_sidechain() {
    for t in [
        BuiltinDeviceType::Gate,
        BuiltinDeviceType::MultibandCompressor,
        BuiltinDeviceType::AutoFilter,
    ] {
        assert_eq!(ether_devices::descriptor(t).sidechain_inputs, 2, "{t:?}");
    }
}

#[test]
fn factory_presets_parse_and_match_their_type() {
    for t in BuiltinDeviceType::ALL {
        for p in ether_devices::factory_presets(t) {
            let preset = load_preset(p.json).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert_eq!(
                preset.device,
                PresetDevice::Builtin { device: t },
                "{}",
                p.id
            );
            assert!(
                p.id.starts_with(&format!("{}/", t.key())),
                "{}: id must be \"{}/<slug>\"",
                p.id,
                t.key()
            );
            let desc = ether_devices::descriptor(t);
            for id in preset.params.keys() {
                assert!(
                    desc.params.iter().any(|q| q.id == *id),
                    "{}: unknown param {id:?}",
                    p.id
                );
            }
            if let Some(rack) = &preset.rack {
                check_rack_preset(p.id, t, rack);
            }
        }
    }
}

/// v0.3: factory rack presets use built-in devices only, that fit the rack
/// (audio effect rack: audio effects, MIDI effect rack: MIDI effects, no nested racks), with
/// known in-range params, and map only automatable params.
fn check_rack_preset(id: &str, rack_type: BuiltinDeviceType, rack: &PresetRack) {
    use ether_core::protocol::devices::DeviceCategory;
    let mut descs = Vec::new();
    for c in &rack.chains {
        let mut row = Vec::new();
        for d in &c.devices {
            let PresetDevice::Builtin { device } = d.device else {
                panic!("{id}: factory rack presets use built-in devices only");
            };
            assert!(
                !device.is_rack() && device != BuiltinDeviceType::DrumRack,
                "{id}: nested rack"
            );
            let desc = ether_devices::descriptor(device);
            match rack_type {
                BuiltinDeviceType::AudioEffectRack => {
                    assert_eq!(desc.category, DeviceCategory::AudioEffect, "{id}")
                }
                BuiltinDeviceType::MidiEffectRack => {
                    assert_eq!(desc.category, DeviceCategory::NoteEffect, "{id}")
                }
                _ => {}
            }
            for (pid, v) in &d.params {
                let info = desc
                    .params
                    .iter()
                    .find(|q| q.id == *pid)
                    .unwrap_or_else(|| panic!("{id}: {device:?} has no param {pid:?}"));
                assert_eq!(
                    info.snap(*v),
                    *v,
                    "{id}: {device:?} param {pid:?} out of range"
                );
            }
            assert!(d.state.is_none() && d.kind.is_none(), "{id}");
            row.push(desc);
        }
        descs.push(row);
    }
    for m in &rack.modulators {
        let desc = ether_devices::modulators::descriptor(m.kind);
        for pid in m.params.keys() {
            assert!(
                desc.params.iter().any(|q| q.id == *pid),
                "{id}: modulator param {pid:?}"
            );
        }
    }
    for m in &rack.mappings {
        let desc = match m.target {
            PresetModTarget::Rack => ether_devices::descriptor(rack_type),
            PresetModTarget::ChainDevice { chain, device } => {
                descs[chain as usize][device as usize].clone()
            }
        };
        let info = desc.params.iter().find(|q| q.id == m.param);
        assert!(
            info.is_some_and(|q| q.automatable),
            "{id}: mapping to {:?} param {:?}",
            m.target,
            m.param
        );
        assert!((-1.0..=1.0).contains(&m.depth), "{id}: depth");
    }
}

/// The native synths share a `Mode` (Poly / Mono / Legato) voice choice and a global
/// `Transpose` (semitones) + `Fine` (cents), each appended after the existing ids (never
/// renumbered): Synth 14 (Transpose 1 / Fine 9 already there), Poly Synth 65/66 (Mode 22
/// already there), FM Synth 76 (Transpose 53 / Fine 54 already there).
#[test]
fn native_synths_have_a_voice_mode_and_global_pitch() {
    use ether_core::protocol::devices::ParamUnit;
    use ether_core::protocol::model::ParamId;
    let cases = [
        (BuiltinDeviceType::Synth, 15, 14, 1, 9, (-24.0, 24.0)),
        (BuiltinDeviceType::PolySynth, 67, 22, 65, 66, (-48.0, 48.0)),
        (BuiltinDeviceType::FmSynth, 77, 76, 53, 54, (-48.0, 48.0)),
    ];
    for (t, count, mode, transpose, fine, range) in cases {
        let d = ether_devices::descriptor(t);
        assert_eq!(d.params.len(), count, "{t:?}");
        for (i, p) in d.params.iter().enumerate() {
            assert_eq!(p.id, ParamId(i as u32), "{t:?}: dense, append-only ids");
        }
        let get = |id: u32| &d.params[id as usize];
        let m = get(mode);
        assert_eq!(m.name, "Mode", "{t:?}");
        assert_eq!(m.group.as_deref(), Some("Voice"), "{t:?}");
        assert_eq!(
            m.labels.as_deref(),
            Some(&["Poly".to_owned(), "Mono".to_owned(), "Legato".to_owned()][..]),
            "{t:?}"
        );
        assert_eq!(
            m.default, 0.0,
            "{t:?}: poly by default (documents sound as before)"
        );
        let tr = get(transpose);
        assert_eq!(
            (tr.name.as_str(), tr.unit, tr.min, tr.max, tr.default),
            ("Transpose", ParamUnit::Semitones, range.0, range.1, 0.0),
            "{t:?}"
        );
        let f = get(fine);
        assert_eq!(
            (f.name.as_str(), f.unit, f.min, f.max, f.default),
            ("Fine", ParamUnit::Cents, -100.0, 100.0, 0.0),
            "{t:?}"
        );
        // All three are automatable and on the device panel.
        let layout = d.layout.as_ref().expect("layout");
        let shown = serde_json::to_string(layout).unwrap();
        for id in [mode, transpose, fine] {
            assert!(get(id).automatable && !get(id).hidden, "{t:?}: {id}");
            assert!(
                [format!("\"param\":{id}}}"), format!("\"param\":{id},")]
                    .iter()
                    .any(|needle| shown.contains(needle.as_str())),
                "{t:?}: param {id} is not on the panel"
            );
        }
    }
}

/// Factory preset text (preset, chain, device and modulator names, descriptions, author) is
/// lowercase like the rest of the UI, acronyms and units included ("lfo", "fm",
/// "db", "hz": no exceptions, docs/LOCALIZATION.md); only note names (A4, C3) keep a capital.
#[test]
fn factory_preset_text_is_lowercase() {
    fn note(w: &str) -> bool {
        let b = w.as_bytes();
        b.len() >= 2
            && (b'A'..=b'G').contains(&b[0])
            && w[1..]
                .trim_start_matches(['#', 'b', '-'])
                .bytes()
                .all(|c| c.is_ascii_digit())
    }
    fn walk(v: &serde_json::Value, id: &str, bad: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    if let (true, Some(text)) = (
                        matches!(k.as_str(), "name" | "description" | "author"),
                        v.as_str(),
                    ) {
                        let words = text.split(|c: char| !c.is_alphanumeric());
                        for w in words.filter(|w| w.chars().any(char::is_uppercase)) {
                            if !note(w) {
                                bad.push(format!("{id}: {k} {text:?} ({w})"));
                            }
                        }
                    }
                    walk(v, id, bad);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, id, bad)),
            _ => {}
        }
    }
    let mut bad = Vec::new();
    for t in BuiltinDeviceType::ALL {
        for p in ether_devices::factory_presets(t) {
            let v: serde_json::Value = serde_json::from_str(p.json).unwrap();
            walk(&v, p.id, &mut bad);
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
