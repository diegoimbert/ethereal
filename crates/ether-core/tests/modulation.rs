//! Engine-side modulation (`racks-modulation`): `base + Σ depth·m` on the automation grid,
//! automation and live changes set the base (sample-accurate, block-size independent),
//! macros, note-triggered envelopes, restore on unmap, readback frames.

mod common;

use common::*;
use ether_core::automation::to_plain;
use ether_core::graph::{AutomationDesc, ParamMapping, RenderGraphDesc, ResolvedTarget, TrackDesc};
use ether_core::modulation::{
    MacroDesc, ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, CurveShape, DeviceId, ModulatorId, ModulatorKind, ParamId, TrackKind, Ulid,
};
use ether_core::{
    AnalysisKind, EventKind, NodeKey, ParamChange, ParamTarget, TransportControl, create,
};

const BEAT: usize = 24_000;
const P: ParamId = ParamId(3);
const LINEAR: ParamMapping = ParamMapping {
    min: 0.0,
    max: 100.0,
    scale: ParamScale::Linear,
    steps: None,
};

fn lfo(host: NodeKey, rate_hz: f64) -> ModulatorDesc {
    ModulatorDesc {
        id: ModulatorId(Ulid(7)),
        host,
        kind: ModulatorKind::Lfo,
        // Shape saw up (phase-linear), rate, no sync.
        params: vec![(ParamId(0), 2.0), (ParamId(1), rate_hz), (ParamId(2), 0.0)],
        sidechain: None,
    }
}

fn mapping(source: ModSourceDesc, node: NodeKey, depth: f64, base: f64) -> ModMappingDesc {
    ModMappingDesc {
        source,
        node,
        param: P,
        depth,
        mapping: LINEAR,
        base,
    }
}

fn desc(version: u64, t: TrackDesc) -> RenderGraphDesc {
    RenderGraphDesc {
        version,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

fn approx(a: &[(u64, f64)], b: &[(u64, f64)]) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        assert!(x.0 == y.0 && (x.1 - y.1).abs() < 1e-9, "{a:?} vs {b:?}");
    }
}

fn params_of(seen: &[Seen]) -> Vec<(u64, f64)> {
    events(seen)
        .into_iter()
        .filter_map(|(t, k)| match k {
            EventKind::Param { param, value } if param == P => Some((t, value)),
            _ => None,
        })
        .collect()
}

/// Renders an LFO-modulated recorder param with automation of its base; returns the
/// param events the node saw.
fn modulated_render(block: usize) -> Vec<(u64, f64)> {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[rec]);
    t.modulation = ModulationDesc {
        modulators: vec![lfo(rec, 3.0)],
        mappings: vec![mapping(ModSourceDesc::Modulator(0), rec, 0.25, 0.5)],
        macros: vec![],
    };
    t.automation = vec![AutomationDesc {
        target: AutomationTarget::DeviceParam {
            device: DeviceId(Ulid(9)),
            param: P,
        },
        resolved: ResolvedTarget::Node {
            node: rec,
            param: P,
        },
        points: vec![
            (0.0, 0.2, CurveShape::Linear),
            (1.0, 0.8, CurveShape::Linear),
            (1.5, 0.3, CurveShape::Step),
        ],
        mapping: LINEAR,
    }];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 2 * BEAT, block);
    params_of(&drain(&mut rx))
}

#[test]
fn lfo_modulates_on_the_grid_around_the_automated_base() {
    let ev = modulated_render(BLOCK);
    assert!(ev.len() > 100);
    for &(t, v) in &ev {
        assert!(t % 32 == 0 || t == 36_000, "off-grid event at {t}");
        assert!((0.0..=100.0).contains(&v));
    }
    // At the step breakpoint (beat 1.5 = 36000) the base jumps to 0.3; the saw adds
    // 0.25 · (2·phase - 1), phase = 3 Hz · t.
    let at = |s: u64| ev.iter().rev().find(|e| e.0 <= s).unwrap().1;
    let phase = |s: u64| (s as f64 * 3.0 / 48_000.0).fract();
    let expect = |s: u64, base: f64| (base + 0.25 * (2.0 * phase(s) - 1.0)).clamp(0.0, 1.0) * 100.0;
    assert!(
        (at(36_000) - expect(36_000, 0.3)).abs() < 1e-6,
        "{}",
        at(36_000)
    );
    // Mid-ramp: base = 0.2 + 0.6 · beat.
    let s = 12_000u64;
    assert!((at(s) - expect(s, 0.2 + 0.6 * 0.5)).abs() < 1e-6);
}

#[test]
fn modulation_is_block_size_independent() {
    let a = modulated_render(64);
    let b = modulated_render(512);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.0, y.0);
        assert!((x.1 - y.1).abs() < 1e-6, "{x:?} {y:?}");
    }
}

#[test]
fn macros_are_sources_and_live_changes_set_the_base() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let rack = p.handle.add_node(Box::new(Dc(0.0))).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[rack, rec]);
    t.modulation = ModulationDesc {
        modulators: vec![],
        mappings: vec![mapping(
            ModSourceDesc::Macro { rack, index: 2 },
            rec,
            -0.5,
            0.8,
        )],
        macros: vec![MacroDesc {
            rack,
            values: [0.0, 0.0, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0],
        }],
    };
    p.handle.publish(desc(1, t)).unwrap();
    render(&mut p.engine, 256, 256);
    // 0.8 - 0.5 · 0.2 = 0.7.
    approx(&params_of(&drain(&mut rx)), &[(0, 70.0)]);
    // Turning the macro: consumed as a macro value.
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rack,
                param: ParamId(2),
            },
            value: 1.0,
        })
        .unwrap();
    render(&mut p.engine, 256, 256);
    approx(&params_of(&drain(&mut rx)), &[(256, 30.0)]);
    // A knob move on the target sets its base.
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rec,
                param: P,
            },
            value: 100.0,
        })
        .unwrap();
    render(&mut p.engine, 256, 256);
    approx(&params_of(&drain(&mut rx)), &[(512, 50.0)]);
}

#[test]
fn envelope_triggers_at_the_note_and_unmapping_restores_the_base() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = vec![midi_clip(cid(1), 0.0, 4.0, &[(0.5, 0.5, 60)])];
    let env = ModulatorDesc {
        id: ModulatorId(Ulid(8)),
        host: rec,
        kind: ModulatorKind::Envelope,
        // Instant attack, long decay to full sustain.
        params: vec![
            (ParamId(0), 0.0),
            (ParamId(1), 100.0),
            (ParamId(2), 100.0),
            (ParamId(3), 50.0),
            (ParamId(4), 0.0),
        ],
        sidechain: None,
    };
    let modded = |t: &TrackDesc| {
        let mut t = t.clone();
        t.modulation = ModulationDesc {
            modulators: vec![env.clone()],
            mappings: vec![mapping(ModSourceDesc::Modulator(0), rec, 0.5, 0.25)],
            macros: vec![],
        };
        t
    };
    p.handle.publish(desc(1, modded(&t))).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, BEAT, BLOCK);
    let ev = params_of(&drain(&mut rx));
    assert_eq!(ev[0], (0, 25.0));
    // Exactly at the note (beat 0.5 = 12000), mid-block, not on the grid.
    assert_eq!(ev[1], (12_000, 75.0), "{ev:?}");
    // Unmapped: the base comes back once.
    p.handle.publish(desc(2, t)).unwrap();
    render(&mut p.engine, 512, BLOCK);
    assert_eq!(params_of(&drain(&mut rx)), vec![(BEAT as u64, 25.0)]);
}

#[test]
fn readback_frames_carry_base_and_effective() {
    let mut p = create(config());
    let (rec, _rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[rec]);
    t.modulation = ModulationDesc {
        modulators: vec![lfo(rec, 1.0)],
        mappings: vec![mapping(ModSourceDesc::Modulator(0), rec, 0.5, 0.5)],
        macros: vec![],
    };
    p.handle.publish(desc(1, t)).unwrap();
    render(&mut p.engine, 4800, BLOCK);
    let mut frames = Vec::new();
    p.handle.poll_analysis(|f| frames.push(*f));
    let m: Vec<_> = frames
        .iter()
        .filter(|f| f.kind == AnalysisKind::Modulation)
        .collect();
    assert!(!m.is_empty());
    let v = m.last().unwrap().values();
    assert_eq!(m.last().unwrap().node, rec);
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].to_bits(), P.0);
    assert_eq!(v[1], 0.5);
    assert!((0.0..=1.0).contains(&v[2]) && v[2] != 0.5);
    let _ = to_plain(&LINEAR, 0.5);
}
