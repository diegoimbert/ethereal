//! Render-graph fixtures for the codec tests and benchmarks (`web-perf`).
//!
//! Also included by `crates/ether-wasm/tests/web_perf*.rs` via `#[path]`, so keep it
//! dependency-free beyond `ether-core`.

#![allow(dead_code, clippy::manual_is_multiple_of)]

use ether_core::NodeKey;
use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, MetronomeDesc, NoteDesc, PadDesc,
    ParamMapping, RackDesc, RenderGraphDesc, ResolvedTarget, SendDesc, TrackDesc, WarpDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, ClipId, CurveShape, DeviceId, DrumPadId, FadeCurve, MediaId, MetronomeSound,
    ParamId, SendId, TempoCurve, TimeSignature, TrackId, TrackKind, Ulid, WarpMode,
};
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};

/// Shape of [`large_project`].
pub const LARGE_TRACKS: usize = 64;
pub const LARGE_CLIPS: usize = 500;

/// Deterministic ids: `(kind << 64) | n`.
fn ulid(kind: u128, n: usize) -> Ulid {
    Ulid((kind << 64) | n as u128)
}

fn key(n: usize) -> NodeKey {
    NodeKey {
        index: n as u32,
        generation: 1 + (n as u32 % 3),
    }
}

fn mapping(i: usize) -> ParamMapping {
    let scale = match i % 4 {
        0 => ParamScale::Linear,
        1 => ParamScale::Log,
        2 => ParamScale::Power { exponent: 2.0 },
        _ => ParamScale::Fader,
    };
    ParamMapping {
        min: if matches!(scale, ParamScale::Log) {
            20.0
        } else {
            -70.0
        },
        max: if matches!(scale, ParamScale::Log) {
            20_000.0
        } else {
            6.0
        },
        scale,
        steps: (i % 5 == 0).then_some(8),
    }
}

fn lane(
    target: AutomationTarget,
    resolved: ResolvedTarget,
    points: usize,
    seed: usize,
) -> AutomationDesc {
    AutomationDesc {
        target,
        resolved,
        points: (0..points)
            .map(|p| {
                let curve = match (p + seed) % 3 {
                    0 => CurveShape::Linear,
                    1 => CurveShape::Step,
                    _ => CurveShape::Curve {
                        tension: ((p as f32) * 0.37).sin(),
                    },
                };
                (
                    p as f64 * 0.5 + (seed % 7) as f64 / 3.0,
                    ((p * 31 + seed) % 101) as f64 / 100.0,
                    curve,
                )
            })
            .collect(),
        mapping: mapping(seed),
    }
}

/// A big arrangement: [`LARGE_TRACKS`] tracks (master, 4 returns, 3 groups, the rest audio
/// and MIDI), 3 devices per track, 2 sends each, [`LARGE_CLIPS`] clips (MIDI with 32 notes,
/// audio with warp markers and fades), 3 automation lanes of 64 points per track, a clip
/// envelope on every 4th clip, a tempo map, and one drum rack with 16 pads.
pub fn large_project() -> RenderGraphDesc {
    const RETURNS: usize = 4;
    const GROUPS: usize = 3;
    let master = TrackId(ulid(1, 0));
    let mut tracks = Vec::with_capacity(LARGE_TRACKS);
    let mut node = 0usize;
    let mut next_node = || {
        node += 1;
        key(node)
    };
    let clips_per_track = LARGE_CLIPS / (LARGE_TRACKS - 1 - RETURNS - GROUPS);
    let mut clips_left = LARGE_CLIPS;
    for t in 0..LARGE_TRACKS {
        let id = TrackId(ulid(1, t));
        let kind = match t {
            0 => TrackKind::Master,
            1..=RETURNS => TrackKind::Return,
            5..=7 => TrackKind::Group,
            t if t % 2 == 0 => TrackKind::Audio,
            _ => TrackKind::Midi,
        };
        let group = (t > 7 && t % 3 == 0).then(|| TrackId(ulid(1, 5 + t % GROUPS)));
        let playable = matches!(kind, TrackKind::Audio | TrackKind::Midi);
        let chain: Vec<ChainEntry> = (0..3)
            .map(|d| ChainEntry {
                node: next_node(),
                enabled: d != 1 || t % 2 == 0,
                sidechain: (d == 2 && t % 8 == 0 && t > 0).then(|| TrackId(ulid(1, t - 1))),
            })
            .collect();
        let sends = if playable {
            (0..2)
                .map(|s| SendDesc {
                    id: SendId(ulid(2, t * 2 + s)),
                    to: TrackId(ulid(1, 1 + (t + s) % RETURNS)),
                    level: 0.25 * (s + 1) as f32,
                    pre_fader: s == 1,
                })
                .collect()
        } else {
            Vec::new()
        };
        let n_clips = if !playable {
            0
        } else if t == LARGE_TRACKS - 1 {
            clips_left
        } else {
            clips_per_track.min(clips_left)
        };
        clips_left -= n_clips;
        let clips = (0..n_clips)
            .map(|c| {
                let n = t * 1000 + c;
                let content = if kind == TrackKind::Midi {
                    ClipContentDesc::Midi {
                        notes: (0..32)
                            .map(|k| NoteDesc {
                                start: k as f64 * 0.25,
                                duration: 0.25 - 1.0 / 64.0,
                                key: 36 + (k * 7 % 48) as u8,
                                velocity: 0.5 + (k % 4) as f32 / 8.0,
                                release_velocity: 0.5,
                            })
                            .collect(),
                    }
                } else {
                    ClipContentDesc::Audio {
                        media: MediaId(ulid(3, n % 40)),
                        gain: 0.8,
                        transpose: (c % 3) as f32 - 1.0,
                        fade_in: 0.01,
                        fade_out: 1.0 / 3.0,
                        warp: (c % 2 == 0).then(|| WarpDesc {
                            mode: if c % 4 == 0 {
                                WarpMode::Complex
                            } else {
                                WarpMode::Repitch
                            },
                            markers: (0..8).map(|m| (m as f64, m as f64 * 0.51)).collect(),
                        }),
                        fade_in_curve: FadeCurve::Linear,
                        fade_out_curve: FadeCurve::Curve { tension: -0.3 },
                        reversed: c % 5 == 0,
                    }
                };
                ClipDesc {
                    id: ClipId(ulid(4, n)),
                    start: c as f64 * 8.0,
                    length: 8.0,
                    offset: 0.0,
                    looping: (c % 3 == 0).then_some((0.0, 4.0)),
                    muted: c % 11 == 0,
                    content,
                    envelopes: if c % 4 == 0 {
                        vec![lane(
                            AutomationTarget::TrackVolume { track: id },
                            ResolvedTarget::TrackVolume,
                            16,
                            n,
                        )]
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();
        let automation = vec![
            lane(
                AutomationTarget::TrackVolume { track: id },
                ResolvedTarget::TrackVolume,
                64,
                t,
            ),
            lane(
                AutomationTarget::TrackPan { track: id },
                ResolvedTarget::TrackPan,
                64,
                t + 1,
            ),
            lane(
                AutomationTarget::DeviceParam {
                    device: DeviceId(ulid(5, t)),
                    param: ParamId(3),
                },
                ResolvedTarget::Node {
                    node: chain[0].node,
                    param: ParamId(3),
                },
                64,
                t + 2,
            ),
        ];
        let racks = if t == 9 {
            vec![RackDesc {
                rack: chain[0].node,
                pads: (0..16)
                    .map(|p| PadDesc {
                        pad: DrumPadId(ulid(6, p)),
                        note: 36 + p as u8,
                        choke_group: (p % 4 == 0).then_some(1),
                        chain: vec![ChainEntry {
                            node: next_node(),
                            enabled: true,
                            sidechain: None,
                        }],
                        volume: 1.0,
                        pan: 0.0,
                        mute: false,
                    })
                    .collect(),
            }]
        } else {
            Vec::new()
        };
        tracks.push(TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id,
            kind,
            chain,
            output: (t != 0).then(|| group.unwrap_or(master)),
            group,
            sends,
            volume: 0.9,
            pan: ((t % 9) as f32 - 4.0) / 4.0,
            mute: t % 13 == 0,
            solo: false,
            audio_input: (kind == TrackKind::Audio && t % 10 == 0).then_some((0, 2)),
            monitor: false,
            armed: false,
            clips,
            automation,
            racks,
        });
    }
    assert_eq!(clips_left, 0);
    RenderGraphDesc {
        vcas: Default::default(),
        version: 1234,
        tempo: vec![
            TempoPointDesc {
                beat: 0.0,
                bpm: 120.0,
                curve: TempoCurve::Step,
            },
            TempoPointDesc {
                beat: 64.0,
                bpm: 96.0,
                curve: TempoCurve::Linear,
            },
            TempoPointDesc {
                beat: 128.0,
                bpm: 140.0,
                curve: TempoCurve::Step,
            },
        ],
        signatures: vec![
            TimeSignatureDesc {
                beat: 0.0,
                signature: TimeSignature {
                    numerator: 4,
                    denominator: 4,
                },
            },
            TimeSignatureDesc {
                beat: 64.0,
                signature: TimeSignature {
                    numerator: 7,
                    denominator: 8,
                },
            },
        ],
        loop_enabled: true,
        loop_start: 16.0,
        loop_end: 48.0,
        metronome: false,
        click: MetronomeDesc {
            volume: 0.5,
            accent: true,
            sound: MetronomeSound::Wood,
            count_in_end: None,
        },
        tracks,
    }
}

/// Every enum variant and `Option` state of every desc type at least once (small).
pub fn all_variants() -> RenderGraphDesc {
    let kinds = [
        TrackKind::Audio,
        TrackKind::Midi,
        TrackKind::Group,
        TrackKind::Return,
        TrackKind::Master,
        TrackKind::Vca,
    ];
    let targets = [
        (
            AutomationTarget::TrackVolume {
                track: TrackId(ulid(1, 0)),
            },
            ResolvedTarget::TrackVolume,
        ),
        (
            AutomationTarget::TrackPan {
                track: TrackId(ulid(1, 0)),
            },
            ResolvedTarget::TrackPan,
        ),
        (
            AutomationTarget::SendLevel {
                send: SendId(ulid(2, 0)),
            },
            ResolvedTarget::Send {
                send: SendId(ulid(2, 0)),
            },
        ),
        (
            AutomationTarget::DeviceParam {
                device: DeviceId(Ulid(u128::MAX)),
                param: ParamId(u32::MAX),
            },
            ResolvedTarget::Node {
                node: NodeKey {
                    index: u32::MAX,
                    generation: 0,
                },
                param: ParamId(u32::MAX),
            },
        ),
    ];
    let fades = [
        FadeCurve::Linear,
        FadeCurve::EqualPower,
        FadeCurve::Curve { tension: -1.0 },
    ];
    let tracks = kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: TrackId(ulid(1, i)),
            kind,
            chain: vec![
                ChainEntry {
                    node: key(i),
                    enabled: true,
                    sidechain: None,
                },
                ChainEntry {
                    node: key(i + 10),
                    enabled: false,
                    sidechain: Some(TrackId(ulid(1, 99))),
                },
            ],
            output: (i % 2 == 0).then(|| TrackId(ulid(1, 4))),
            group: (i % 2 == 1).then(|| TrackId(ulid(1, 2))),
            sends: vec![SendDesc {
                id: SendId(ulid(2, i)),
                to: TrackId(ulid(1, 3)),
                level: -0.0,
                pre_fader: i % 2 == 0,
            }],
            volume: f32::MIN_POSITIVE / 2.0,
            pan: -1.0,
            mute: i % 2 == 0,
            solo: i % 2 == 1,
            audio_input: (i == 0).then_some((u16::MAX, 1)),
            monitor: i == 1,
            armed: i == 2,
            clips: vec![
                ClipDesc {
                    id: ClipId(ulid(4, i * 2)),
                    start: 1.0 / 3.0,
                    length: f64::MAX,
                    offset: f64::MIN_POSITIVE,
                    looping: Some((0.0, 1.0)),
                    muted: true,
                    content: ClipContentDesc::Midi {
                        notes: vec![NoteDesc {
                            start: 0.1,
                            duration: 0.2,
                            key: 127,
                            velocity: 1.0,
                            release_velocity: 0.0,
                        }],
                    },
                    envelopes: vec![],
                },
                ClipDesc {
                    id: ClipId(ulid(4, i * 2 + 1)),
                    start: 0.0,
                    length: 4.0,
                    offset: -0.0,
                    looping: None,
                    muted: false,
                    content: ClipContentDesc::Audio {
                        media: MediaId(ulid(3, i)),
                        gain: 2.0,
                        transpose: -12.0,
                        fade_in: 0.0,
                        fade_out: 0.5,
                        warp: match i % 3 {
                            0 => None,
                            1 => Some(WarpDesc {
                                mode: WarpMode::Repitch,
                                markers: vec![(0.0, 0.0), (4.0, 2.0)],
                            }),
                            _ => Some(WarpDesc {
                                mode: WarpMode::Complex,
                                markers: vec![],
                            }),
                        },
                        fade_in_curve: fades[i % 3],
                        fade_out_curve: fades[(i + 1) % 3],
                        reversed: i % 2 == 0,
                    },
                    envelopes: vec![AutomationDesc {
                        target: targets[i % 4].0,
                        resolved: targets[i % 4].1,
                        points: vec![(0.0, 1.0, CurveShape::Step)],
                        mapping: mapping(i + 1),
                    }],
                },
            ],
            automation: targets
                .iter()
                .enumerate()
                .map(|(j, &(target, resolved))| AutomationDesc {
                    target,
                    resolved,
                    points: vec![
                        (0.0, 0.0, CurveShape::Linear),
                        (1.0, 0.5, CurveShape::Step),
                        (2.0, 1.0, CurveShape::Curve { tension: 0.5 }),
                    ],
                    mapping: mapping(i + j),
                })
                .collect(),
            racks: if i == 1 {
                vec![RackDesc {
                    rack: key(i),
                    pads: vec![
                        PadDesc {
                            pad: DrumPadId(ulid(6, 0)),
                            note: 36,
                            choke_group: Some(2),
                            chain: vec![],
                            volume: 0.5,
                            pan: 0.25,
                            mute: true,
                        },
                        PadDesc {
                            pad: DrumPadId(ulid(6, 1)),
                            note: 38,
                            choke_group: None,
                            chain: vec![ChainEntry {
                                node: key(50),
                                enabled: true,
                                sidechain: None,
                            }],
                            volume: 1.0,
                            pan: 0.0,
                            mute: false,
                        },
                    ],
                }]
            } else {
                vec![]
            },
        })
        .collect();
    RenderGraphDesc {
        vcas: Default::default(),
        version: u64::MAX,
        tempo: vec![
            TempoPointDesc {
                beat: 0.0,
                bpm: 120.0,
                curve: TempoCurve::Step,
            },
            TempoPointDesc {
                beat: 8.0,
                bpm: 60.5,
                curve: TempoCurve::Linear,
            },
        ],
        signatures: vec![TimeSignatureDesc {
            beat: 0.0,
            signature: TimeSignature {
                numerator: 3,
                denominator: 4,
            },
        }],
        loop_enabled: true,
        loop_start: 0.0,
        loop_end: f64::INFINITY,
        metronome: true,
        click: MetronomeDesc {
            volume: 0.75,
            accent: false,
            sound: MetronomeSound::Beep,
            count_in_end: Some(4.0),
        },
        tracks,
    }
}

/// Fill the v0.2 track fields (contracts-3) and `vcas` of `d`, covering every variant and
/// `Option` state. Only finite floats (`frozen` still travels as a JSON blob).
pub fn fill_v02(d: &mut RenderGraphDesc) {
    use ether_core::InputTapDesc;
    use ether_core::freeze::FrozenDesc;
    use ether_core::modulation::{
        MacroDesc, ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc,
    };
    use ether_core::protocol::model::{InputTap, ModulatorId, ModulatorKind, RackChainId};
    use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
    use ether_core::vca::VcaDesc;
    let taps = [InputTap::PreFx, InputTap::PostFx, InputTap::PostFader];
    let racks = [
        ChainRackKind::Instrument,
        ChainRackKind::AudioEffect,
        ChainRackKind::MidiEffect,
    ];
    let n = d.tracks.len();
    for (i, t) in d.tracks.iter_mut().enumerate() {
        t.frozen = (i % 2 == 0).then(|| FrozenDesc {
            media: MediaId(ulid(9, i)),
            start_seconds: 0.25 * i as f64,
        });
        t.input_tap = (i % 2 == 1).then(|| InputTapDesc {
            track: TrackId(ulid(1, (i + 1) % n)),
            point: taps[i % 3],
        });
        t.vca = (i % 3 == 0).then(|| TrackId(ulid(10, 0)));
        t.chain_racks = vec![ChainRackDesc {
            rack: key(i + 20),
            kind: racks[i % 3],
            chains: vec![RackChainDesc {
                id: RackChainId(ulid(11, i)),
                chain: vec![ChainEntry {
                    node: key(i + 30),
                    enabled: i % 2 == 0,
                    sidechain: None,
                }],
                volume: 0.5,
                pan: -0.25,
                mute: i % 2 == 1,
                keys: (0, 127),
                velocities: (1, 100),
                select: (i as u8, 127),
            }],
            selector: (i * 7 % 128) as u8,
        }];
        t.modulation = ModulationDesc {
            modulators: ModulatorKind::ALL
                .iter()
                .enumerate()
                .map(|(k, &kind)| ModulatorDesc {
                    id: ModulatorId(ulid(12, i * 10 + k)),
                    host: key(i),
                    kind,
                    params: vec![(ParamId(k as u32), 1.0 / 3.0)],
                    sidechain: (kind == ModulatorKind::EnvelopeFollower)
                        .then(|| TrackId(ulid(1, (i + 2) % n))),
                })
                .collect(),
            mappings: vec![
                ModMappingDesc {
                    source: ModSourceDesc::Modulator(0),
                    node: key(i),
                    param: ParamId(3),
                    depth: -0.5,
                    mapping: mapping(i),
                    base: 0.125,
                },
                ModMappingDesc {
                    source: ModSourceDesc::Macro {
                        rack: key(i + 20),
                        index: 7,
                    },
                    node: key(i + 30),
                    param: ParamId(0),
                    depth: 1.0,
                    mapping: mapping(i + 1),
                    base: 0.0,
                },
            ],
            macros: vec![MacroDesc {
                rack: key(i + 20),
                values: [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 1.0],
            }],
        };
    }
    d.vcas = vec![
        VcaDesc {
            id: TrackId(ulid(10, 0)),
            volume: 0.5,
            mute: false,
            parent: Some(TrackId(ulid(10, 1))),
            automation: vec![AutomationDesc {
                target: AutomationTarget::TrackVolume {
                    track: TrackId(ulid(10, 0)),
                },
                resolved: ResolvedTarget::TrackVolume,
                points: vec![
                    (0.0, 0.25, CurveShape::Linear),
                    (4.0, 1.0, CurveShape::Curve { tension: -0.5 }),
                    (8.0, 0.0, CurveShape::Step),
                ],
                mapping: mapping(3),
            }],
        },
        VcaDesc {
            id: TrackId(ulid(10, 1)),
            volume: 1.0,
            mute: true,
            parent: None,
            automation: vec![],
        },
    ];
}

/// [`all_variants`] with the v0.2 fields filled ([`fill_v02`]).
pub fn v02_filled() -> RenderGraphDesc {
    let mut d = all_variants();
    fill_v02(&mut d);
    d
}

/// [`large_project`] with the v0.2 fields filled on every track.
pub fn large_v02_project() -> RenderGraphDesc {
    let mut d = large_project();
    fill_v02(&mut d);
    d
}

/// The remaining `MetronomeSound` variants (`all_variants` uses `Beep`).
pub fn metronome_sounds() -> [MetronomeSound; 3] {
    [
        MetronomeSound::Classic,
        MetronomeSound::Wood,
        MetronomeSound::Beep,
    ]
}

#[test]
fn large_project_has_the_documented_shape() {
    let d = large_project();
    assert_eq!(d.tracks.len(), LARGE_TRACKS);
    assert_eq!(
        d.tracks.iter().map(|t| t.clips.len()).sum::<usize>(),
        LARGE_CLIPS
    );
}
