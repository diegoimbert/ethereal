//! `BinaryCodec` (web-perf): exact round trips (vectors + proptest over every desc type),
//! versioning and malformed input.

#[path = "codec_fixture.rs"]
mod fixture;

use ether_core::InputTapDesc;
use ether_core::NodeKey;
use ether_core::codec::{BinaryCodec, CodecError, GraphCodec};
use ether_core::freeze::FrozenDesc;
use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, MetronomeDesc, NoteDesc, PadDesc,
    ParamMapping, RackDesc, RenderGraphDesc, ResolvedTarget, SendDesc, TrackDesc, WarpDesc,
};
use ether_core::modulation::{ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, ClipId, CurveShape, DeviceId, DrumPadId, FadeCurve, MediaId, MetronomeSound,
    ParamId, SendId, TempoCurve, TimeSignature, TrackId, TrackKind, Ulid, WarpMode,
};
use ether_core::protocol::model::{InputTap, ModulatorId, ModulatorKind, RackChainId};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};
use ether_core::vca::VcaDesc;
use proptest::collection::vec;
use proptest::option;
use proptest::prelude::*;

fn encode(d: &RenderGraphDesc) -> Vec<u8> {
    let mut out = Vec::new();
    BinaryCodec.encode(d, &mut out);
    out
}

fn roundtrip(d: &RenderGraphDesc) {
    let bytes = encode(d);
    assert_eq!(bytes[0], BinaryCodec::VERSION);
    let back = BinaryCodec.decode(&bytes).expect("decodes");
    assert_eq!(&back, d);
    assert_eq!(encode(&back), bytes, "re-encoding is byte-identical");
}

#[test]
fn default_desc_roundtrips() {
    roundtrip(&RenderGraphDesc::default());
}

#[test]
fn every_variant_roundtrips() {
    let mut d = fixture::all_variants();
    roundtrip(&d);
    for sound in fixture::metronome_sounds() {
        d.click.sound = sound;
        d.click.count_in_end = None;
        roundtrip(&d);
    }
}

/// The v0.2 fields (contracts-3; `input_tap`, `vca` and `vcas` binary since codec v3, the
/// others a JSON blob): every variant and `Option` state.
#[test]
fn v02_fields_roundtrip() {
    let d = fixture::v02_filled();
    assert!(d.tracks.iter().any(|t| t.kind == TrackKind::Vca));
    roundtrip(&d);
    roundtrip(&fixture::large_v02_project());
}

#[test]
fn large_project_roundtrips_and_is_smaller_than_json() {
    let d = fixture::large_project();
    roundtrip(&d);
    let json = serde_json::to_vec(&d).unwrap();
    let bin = encode(&d);
    assert!(
        bin.len() * 2 < json.len(),
        "binary {} vs json {}",
        bin.len(),
        json.len()
    );
}

#[test]
fn encode_appends() {
    let d = fixture::all_variants();
    let mut out = vec![0xAA, 0xBB];
    BinaryCodec.encode(&d, &mut out);
    assert_eq!(&out[..2], &[0xAA, 0xBB]);
    assert_eq!(BinaryCodec.decode(&out[2..]).unwrap(), d);
}

#[test]
fn floats_are_bit_exact() {
    let specials = [
        -0.0,
        f64::MIN_POSITIVE / 3.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7FF8_0000_DEAD_BEEF),
        f64::EPSILON,
    ];
    for v in specials {
        let d = RenderGraphDesc {
            loop_start: v,
            click: MetronomeDesc {
                volume: f32::from_bits(0xFFC0_1234),
                ..Default::default()
            },
            ..Default::default()
        };
        let back = BinaryCodec.decode(&encode(&d)).unwrap();
        assert_eq!(back.loop_start.to_bits(), v.to_bits());
        assert_eq!(back.click.volume.to_bits(), 0xFFC0_1234);
    }
}

#[test]
fn other_versions_are_rejected() {
    let mut bytes = encode(&fixture::all_variants());
    for v in (0..=u8::MAX).filter(|&v| v != BinaryCodec::VERSION) {
        bytes[0] = v;
        assert_eq!(BinaryCodec.decode(&bytes), Err(CodecError::Version(v)));
    }
}

#[test]
fn every_truncation_is_an_error() {
    let bytes = encode(&fixture::all_variants());
    for n in 0..bytes.len() {
        assert!(
            matches!(
                BinaryCodec.decode(&bytes[..n]),
                Err(CodecError::Malformed(_))
            ),
            "prefix of {n} bytes"
        );
    }
}

#[test]
fn trailing_bytes_and_bad_tags_are_errors() {
    let mut bytes = encode(&RenderGraphDesc::default());
    bytes.push(0);
    assert!(matches!(
        BinaryCodec.decode(&bytes),
        Err(CodecError::Malformed(_))
    ));
    // Default layout: version, desc.version u64, tempo count, signatures count, then the
    // `loop_enabled` bool.
    let mut bytes = encode(&RenderGraphDesc::default());
    bytes[1 + 8 + 4 + 4] = 2;
    assert!(matches!(
        BinaryCodec.decode(&bytes),
        Err(CodecError::Malformed(_))
    ));
}

#[test]
fn huge_counts_do_not_allocate() {
    // A tempo count of u32::MAX with no data must fail before reserving anything.
    let mut bytes = vec![BinaryCodec::VERSION];
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        BinaryCodec.decode(&bytes),
        Err(CodecError::Malformed("count exceeds data".into()))
    );
}

// ---- proptest strategies (one per desc type; `nan` allows NaN floats) ----

fn f64s(nan: bool) -> BoxedStrategy<f64> {
    if nan {
        any::<f64>().boxed()
    } else {
        prop_oneof![
            -1e6..1e6f64,
            Just(-0.0),
            Just(f64::INFINITY),
            Just(f64::MIN_POSITIVE / 2.0),
            (-1e300..1e300f64),
        ]
        .boxed()
    }
}

fn f32s(nan: bool) -> BoxedStrategy<f32> {
    if nan {
        any::<f32>().boxed()
    } else {
        prop_oneof![-10.0..10.0f32, Just(-0.0f32), Just(f32::NEG_INFINITY)].boxed()
    }
}

fn ulid() -> impl Strategy<Value = Ulid> {
    any::<u128>().prop_map(Ulid)
}

fn track_id() -> impl Strategy<Value = TrackId> {
    ulid().prop_map(TrackId)
}

fn node_key() -> impl Strategy<Value = NodeKey> {
    (any::<u32>(), any::<u32>()).prop_map(|(index, generation)| NodeKey { index, generation })
}

fn chain_entry() -> impl Strategy<Value = ChainEntry> {
    (node_key(), any::<bool>(), option::of(track_id())).prop_map(|(node, enabled, sidechain)| {
        ChainEntry {
            node,
            enabled,
            sidechain,
        }
    })
}

fn curve_shape(nan: bool) -> impl Strategy<Value = CurveShape> {
    prop_oneof![
        Just(CurveShape::Linear),
        Just(CurveShape::Step),
        f32s(nan).prop_map(|tension| CurveShape::Curve { tension }),
    ]
}

fn fade_curve(nan: bool) -> impl Strategy<Value = FadeCurve> {
    prop_oneof![
        Just(FadeCurve::Linear),
        Just(FadeCurve::EqualPower),
        f32s(nan).prop_map(|tension| FadeCurve::Curve { tension }),
    ]
}

fn automation(nan: bool) -> impl Strategy<Value = AutomationDesc> {
    let target = prop_oneof![
        track_id().prop_map(|track| AutomationTarget::TrackVolume { track }),
        track_id().prop_map(|track| AutomationTarget::TrackPan { track }),
        ulid().prop_map(|u| AutomationTarget::SendLevel { send: SendId(u) }),
        (ulid(), any::<u32>()).prop_map(|(d, p)| AutomationTarget::DeviceParam {
            device: DeviceId(d),
            param: ParamId(p),
        }),
    ];
    let resolved = prop_oneof![
        Just(ResolvedTarget::TrackVolume),
        Just(ResolvedTarget::TrackPan),
        ulid().prop_map(|u| ResolvedTarget::Send { send: SendId(u) }),
        (node_key(), any::<u32>()).prop_map(|(node, p)| ResolvedTarget::Node {
            node,
            param: ParamId(p),
        }),
    ];
    let scale = prop_oneof![
        Just(ParamScale::Linear),
        Just(ParamScale::Log),
        f64s(nan).prop_map(|exponent| ParamScale::Power { exponent }),
        Just(ParamScale::Fader),
    ];
    let mapping = (f64s(nan), f64s(nan), scale, option::of(any::<u32>())).prop_map(
        |(min, max, scale, steps)| ParamMapping {
            min,
            max,
            scale,
            steps,
        },
    );
    (
        target,
        resolved,
        vec((f64s(nan), f64s(nan), curve_shape(nan)), 0..6),
        mapping,
    )
        .prop_map(|(target, resolved, points, mapping)| AutomationDesc {
            target,
            resolved,
            points,
            mapping,
        })
}

fn clip(nan: bool) -> impl Strategy<Value = ClipDesc> {
    let note = (f64s(nan), f64s(nan), any::<u8>(), f32s(nan), f32s(nan)).prop_map(
        |(start, duration, key, velocity, release_velocity)| NoteDesc {
            start,
            duration,
            key,
            velocity,
            release_velocity,
        },
    );
    let warp = (
        prop_oneof![Just(WarpMode::Repitch), Just(WarpMode::Complex)],
        vec((f64s(nan), f64s(nan)), 0..4),
    )
        .prop_map(|(mode, markers)| WarpDesc { mode, markers });
    let audio = (
        ulid(),
        f32s(nan),
        f32s(nan),
        f64s(nan),
        f64s(nan),
        option::of(warp),
        fade_curve(nan),
        fade_curve(nan),
        any::<bool>(),
    )
        .prop_map(
            |(media, gain, transpose, fade_in, fade_out, warp, fi, fo, reversed)| {
                ClipContentDesc::Audio {
                    media: MediaId(media),
                    gain,
                    transpose,
                    fade_in,
                    fade_out,
                    warp,
                    fade_in_curve: fi,
                    fade_out_curve: fo,
                    reversed,
                }
            },
        );
    let content = prop_oneof![
        vec(note, 0..5).prop_map(|notes| ClipContentDesc::Midi { notes }),
        audio,
    ];
    (
        ulid(),
        (f64s(nan), f64s(nan), f64s(nan)),
        option::of((f64s(nan), f64s(nan))),
        any::<bool>(),
        content,
        vec(automation(nan), 0..2),
    )
        .prop_map(
            |(id, (start, length, offset), looping, muted, content, envelopes)| ClipDesc {
                id: ClipId(id),
                start,
                length,
                offset,
                looping,
                muted,
                content,
                envelopes,
            },
        )
}

fn rack(nan: bool) -> impl Strategy<Value = RackDesc> {
    let pad = (
        ulid(),
        any::<u8>(),
        option::of(any::<u8>()),
        vec(chain_entry(), 0..3),
        (f32s(nan), f32s(nan), any::<bool>()),
    )
        .prop_map(
            |(pad, note, choke_group, chain, (volume, pan, mute))| PadDesc {
                pad: DrumPadId(pad),
                note,
                choke_group,
                chain,
                volume,
                pan,
                mute,
            },
        );
    (node_key(), vec(pad, 0..3)).prop_map(|(rack, pads)| RackDesc { rack, pads })
}

fn track(nan: bool) -> impl Strategy<Value = TrackDesc> {
    let kind = prop_oneof![
        Just(TrackKind::Audio),
        Just(TrackKind::Midi),
        Just(TrackKind::Group),
        Just(TrackKind::Return),
        Just(TrackKind::Master),
        Just(TrackKind::Vca),
    ];
    let send =
        (ulid(), track_id(), f32s(nan), any::<bool>()).prop_map(|(id, to, level, pre_fader)| {
            SendDesc {
                id: SendId(id),
                to,
                level,
                pre_fader,
            }
        });
    (
        (track_id(), kind, vec(chain_entry(), 0..4)),
        (option::of(track_id()), option::of(track_id())),
        vec(send, 0..3),
        (f32s(nan), f32s(nan), any::<[bool; 4]>()),
        option::of((any::<u16>(), any::<u16>())),
        vec(clip(nan), 0..3),
        vec(automation(nan), 0..3),
        vec(rack(nan), 0..2),
        track_ext(),
    )
        .prop_map(
            |(
                (id, kind, chain),
                (output, group),
                sends,
                (volume, pan, [mute, solo, monitor, armed]),
                audio_input,
                clips,
                automation,
                racks,
                (frozen, chain_racks, modulation, input_tap, vca),
            )| TrackDesc {
                modulation,
                vca,
                chain_racks,
                frozen,
                input_tap,
                id,
                kind,
                chain,
                output,
                group,
                sends,
                volume,
                pan,
                mute,
                solo,
                audio_input,
                monitor,
                armed,
                clips,
                automation,
                racks,
            },
        )
}

/// Finite floats only: the v0.2 fields travel as a JSON blob (codec v2), which carries no
/// NaN/infinity (the controller never produces them there).
fn finite64() -> BoxedStrategy<f64> {
    prop_oneof![
        -1e6..1e6f64,
        Just(-0.0),
        Just(f64::MIN_POSITIVE / 2.0),
        -1e300..1e300f64
    ]
    .boxed()
}

fn finite32() -> BoxedStrategy<f32> {
    prop_oneof![-10.0..10.0f32, Just(-0.0f32), Just(f32::MIN_POSITIVE / 2.0)].boxed()
}

fn finite_mapping() -> impl Strategy<Value = ParamMapping> {
    let scale = prop_oneof![
        Just(ParamScale::Linear),
        Just(ParamScale::Log),
        finite64().prop_map(|exponent| ParamScale::Power { exponent }),
        Just(ParamScale::Fader),
    ];
    (finite64(), finite64(), scale, option::of(any::<u32>())).prop_map(
        |(min, max, scale, steps)| ParamMapping {
            min,
            max,
            scale,
            steps,
        },
    )
}

type TrackExt = (
    Option<FrozenDesc>,
    Vec<ChainRackDesc>,
    ModulationDesc,
    Option<InputTapDesc>,
    Option<TrackId>,
);

/// The v0.2 track fields (contracts-3).
fn track_ext() -> impl Strategy<Value = TrackExt> {
    let frozen = (ulid(), finite64()).prop_map(|(m, start_seconds)| FrozenDesc {
        media: MediaId(m),
        start_seconds,
    });
    let chain = (
        ulid(),
        vec(chain_entry(), 0..3),
        (finite32(), finite32(), any::<bool>()),
        any::<[u8; 6]>(),
    )
        .prop_map(|(id, chain, (volume, pan, mute), z)| RackChainDesc {
            id: RackChainId(id),
            chain,
            volume,
            pan,
            mute,
            keys: (z[0], z[1]),
            velocities: (z[2], z[3]),
            select: (z[4], z[5]),
        });
    let rack_kind = prop_oneof![
        Just(ChainRackKind::Instrument),
        Just(ChainRackKind::AudioEffect),
        Just(ChainRackKind::MidiEffect),
    ];
    let rack = (node_key(), rack_kind, vec(chain, 0..3))
        .prop_map(|(rack, kind, chains)| ChainRackDesc { rack, kind, chains });
    let kind = prop::sample::select(ModulatorKind::ALL.to_vec());
    let modulator = (
        ulid(),
        node_key(),
        kind,
        vec((any::<u32>(), finite64()), 0..3),
        option::of(track_id()),
    )
        .prop_map(|(id, host, kind, params, sidechain)| ModulatorDesc {
            id: ModulatorId(id),
            host,
            kind,
            params: params.into_iter().map(|(p, v)| (ParamId(p), v)).collect(),
            sidechain,
        });
    let source = prop_oneof![
        any::<u32>().prop_map(ModSourceDesc::Modulator),
        (node_key(), any::<u8>()).prop_map(|(rack, index)| ModSourceDesc::Macro { rack, index }),
    ];
    let mapping = (
        source,
        node_key(),
        any::<u32>(),
        finite64(),
        finite_mapping(),
        finite64(),
    )
        .prop_map(|(source, node, p, depth, mapping, base)| ModMappingDesc {
            source,
            node,
            param: ParamId(p),
            depth,
            mapping,
            base,
        });
    let modulation =
        (vec(modulator, 0..3), vec(mapping, 0..3)).prop_map(|(modulators, mappings)| {
            ModulationDesc {
                modulators,
                mappings,
            }
        });
    let tap = prop_oneof![
        Just(InputTap::PreFx),
        Just(InputTap::PostFx),
        Just(InputTap::PostFader)
    ];
    let input_tap = (track_id(), tap).prop_map(|(track, point)| InputTapDesc { track, point });
    (
        option::of(frozen),
        vec(rack, 0..2),
        modulation,
        option::of(input_tap),
        option::of(track_id()),
    )
}

/// VCAs (binary since codec v3, `groups-buses`: NaN payloads round-trip too).
fn vca(nan: bool) -> impl Strategy<Value = VcaDesc> {
    (
        track_id(),
        f32s(nan),
        any::<bool>(),
        option::of(track_id()),
        vec(automation(nan), 0..3),
    )
        .prop_map(|(id, volume, mute, parent, automation)| VcaDesc {
            id,
            volume,
            mute,
            parent,
            automation,
        })
}

fn desc(nan: bool) -> impl Strategy<Value = RenderGraphDesc> {
    let tempo = (
        f64s(nan),
        f64s(nan),
        prop_oneof![Just(TempoCurve::Step), Just(TempoCurve::Linear)],
    )
        .prop_map(|(beat, bpm, curve)| TempoPointDesc { beat, bpm, curve });
    let signature =
        (f64s(nan), any::<u8>(), any::<u8>()).prop_map(|(beat, numerator, denominator)| {
            TimeSignatureDesc {
                beat,
                signature: TimeSignature {
                    numerator,
                    denominator,
                },
            }
        });
    let click = (
        f32s(nan),
        any::<bool>(),
        prop_oneof![
            Just(MetronomeSound::Classic),
            Just(MetronomeSound::Wood),
            Just(MetronomeSound::Beep),
        ],
        option::of(f64s(nan)),
    )
        .prop_map(|(volume, accent, sound, count_in_end)| MetronomeDesc {
            volume,
            accent,
            sound,
            count_in_end,
        });
    (
        any::<u64>(),
        vec(tempo, 0..3),
        vec(signature, 0..3),
        (any::<bool>(), f64s(nan), f64s(nan), any::<bool>()),
        click,
        vec(track(nan), 0..4),
        vec(vca(nan), 0..3),
    )
        .prop_map(
            |(
                version,
                tempo,
                signatures,
                (loop_enabled, loop_start, loop_end, metronome),
                click,
                tracks,
                vcas,
            )| {
                RenderGraphDesc {
                    vcas,
                    version,
                    tempo,
                    signatures,
                    loop_enabled,
                    loop_start,
                    loop_end,
                    metronome,
                    click,
                    tracks,
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `decode(encode(d)) == d` (NaN-free, so `PartialEq` is meaningful).
    #[test]
    fn prop_roundtrip(d in desc(false)) {
        let bytes = encode(&d);
        prop_assert_eq!(BinaryCodec.decode(&bytes).unwrap(), d);
    }

    /// With arbitrary floats (NaN payloads included) the round trip is bit-exact.
    #[test]
    fn prop_roundtrip_bits(d in desc(true)) {
        let bytes = encode(&d);
        let back = BinaryCodec.decode(&bytes).unwrap();
        prop_assert_eq!(encode(&back), bytes);
    }

    /// Arbitrary corruption never panics (errors or some other valid desc).
    #[test]
    fn prop_corruption_never_panics(d in desc(false), at in any::<prop::sample::Index>(), byte in any::<u8>()) {
        let mut bytes = encode(&d);
        let i = at.index(bytes.len());
        bytes[i] = byte;
        let _ = BinaryCodec.decode(&bytes);
    }
}
