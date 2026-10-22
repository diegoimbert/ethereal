//! Modulated device on/off (`DEVICE_ENABLED_PARAM`, CONTRACTS.md §12.6.1): a rack macro
//! (or a modulator) switches a device on the sample its value crosses 0.5, with the same
//! crossfade (no click), MIDI thru and note releases as the automated switch; the base is
//! the static switch or its automation.

use crate::common;

use common::*;
use ether_core::graph::{AutomationDesc, ParamMapping, RenderGraphDesc, ResolvedTarget, TrackDesc};
use ether_core::modulation::{
    MacroDesc, ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, CurveShape, DEVICE_ENABLED_PARAM, ModulatorId, ModulatorKind, ParamId,
    TrackId, TrackKind, Ulid,
};
use ether_core::{
    AnalysisKind, AudioBuffers, EventKind, Node, NodeKey, ParamChange, ParamTarget, PrepareConfig,
    ProcessContext, ProcessStatus, TransportControl, create,
};

const BEAT: usize = 24_000; // 120 BPM @ 48 kHz
/// `SWITCH_FADE_MS` (10 ms) at 48 kHz.
const FADE: usize = 480;
/// Divides no beat: the switches land inside blocks.
const ODD_BLOCK: usize = 377;
const SWITCH: ParamMapping = ParamMapping {
    min: 0.0,
    max: 1.0,
    scale: ParamScale::Linear,
    steps: Some(2),
};
const UNIT: ParamMapping = ParamMapping {
    min: 0.0,
    max: 1.0,
    scale: ParamScale::Linear,
    steps: None,
};

fn desc(version: u64, t: TrackDesc) -> RenderGraphDesc {
    RenderGraphDesc {
        version,
        tracks: vec![master(), t],
        ..Default::default()
    }
}

/// `source` switches `node` (document switch `on`) with `depth`.
fn switch_mapping(source: ModSourceDesc, node: NodeKey, depth: f64, on: bool) -> ModMappingDesc {
    ModMappingDesc {
        source,
        node,
        param: DEVICE_ENABLED_PARAM,
        depth,
        mapping: SWITCH,
        base: if on { 1.0 } else { 0.0 },
    }
}

/// Step automation of `resolved` through `points` (beat, value).
fn steps(resolved: ResolvedTarget, mapping: ParamMapping, points: &[(f64, f64)]) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume {
            track: TrackId(Ulid(2)),
        },
        resolved,
        points: points
            .iter()
            .map(|&(b, v)| (b, v, CurveShape::Step))
            .collect(),
        mapping,
    }
}

fn macro_lane(rack: NodeKey, points: &[(f64, f64)]) -> AutomationDesc {
    steps(
        ResolvedTarget::Node {
            node: rack,
            param: ParamId(0),
        },
        UNIT,
        points,
    )
}

fn max_step(l: &[f32]) -> f32 {
    l.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// rack → Dc(0.5) → Dc(0.25) (the switched one): 0.25 on, 0.5 off.
fn dc_track(p: &mut ether_core::EngineParts) -> (TrackDesc, NodeKey, NodeKey) {
    let rack = p.handle.add_node(Box::new(Dc(0.0))).unwrap();
    let a = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let b = p.handle.add_node(Box::new(Dc(0.25))).unwrap();
    let t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rack, a, b]);
    (t, rack, b)
}

fn macros(rack: NodeKey, m0: f64) -> Vec<MacroDesc> {
    vec![MacroDesc {
        rack,
        values: [m0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        first: 0,
    }]
}

#[test]
fn a_macro_switches_a_device_on_the_sample_without_a_click() {
    let run = |block: usize| {
        let mut p = create(config());
        let (mut t, rack, b) = dc_track(&mut p);
        // On in the document; the macro turns it off above 0.5 (macro automation: up at
        // beat 1, down at beat 2).
        t.modulation = ModulationDesc {
            modulators: vec![],
            mappings: vec![switch_mapping(
                ModSourceDesc::Macro { rack, index: 0 },
                b,
                -1.0,
                true,
            )],
            macros: macros(rack, 0.0),
        };
        t.automation = vec![macro_lane(rack, &[(0.0, 0.0), (1.0, 0.6), (2.0, 0.4)])];
        p.handle.publish(desc(1, t)).unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        render(&mut p.engine, 3 * BEAT, block).0
    };
    let l = run(ODD_BLOCK);
    assert_eq!(l[BEAT - 1], 0.25);
    assert!(
        l[BEAT] > 0.25 && l[BEAT] < 0.26,
        "off on the macro's sample: {}",
        l[BEAT]
    );
    assert!((l[BEAT + FADE] - 0.5).abs() < 1e-6, "{}", l[BEAT + FADE]);
    assert!((l[2 * BEAT - 1] - 0.5).abs() < 1e-6);
    assert!(l[2 * BEAT] < 0.5 && l[2 * BEAT] > 0.49, "{}", l[2 * BEAT]);
    assert!((l[2 * BEAT + FADE] - 0.25).abs() < 1e-6);
    assert!(
        max_step(&l) <= 0.25 / FADE as f32 * 1.01,
        "click: {}",
        max_step(&l)
    );
    // Block-size independent.
    let other = run(512);
    for (i, (x, y)) in l.iter().zip(&other).enumerate() {
        assert!((x - y).abs() < 1e-6, "sample {i}: {x} != {y}");
    }
}

#[test]
fn a_live_macro_turn_switches_at_the_next_block_and_a_positive_depth_turns_it_on() {
    let mut p = create(config());
    let (mut t, rack, b) = dc_track(&mut p);
    // Off in the document: the macro turns it on from 0.5.
    t.chain[2].enabled = false;
    t.modulation = ModulationDesc {
        modulators: vec![],
        mappings: vec![switch_mapping(
            ModSourceDesc::Macro { rack, index: 0 },
            b,
            1.0,
            false,
        )],
        macros: macros(rack, 0.2),
    };
    p.handle.publish(desc(1, t)).unwrap();
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    assert_eq!(l[1023], 0.5, "off: 0 + 0.2 < 0.5");
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rack,
                param: ParamId(0),
            },
            value: 0.5,
        })
        .unwrap();
    let (l, _) = render(&mut p.engine, 2048, BLOCK);
    assert!(
        l[0] < 0.5 && l[0] > 0.49,
        "fades in from the block start: {}",
        l[0]
    );
    assert!(
        (l[FADE] - 0.25).abs() < 1e-6,
        "on at exactly 0.5: {}",
        l[FADE]
    );
    assert!(
        max_step(&l) <= 0.25 / FADE as f32 * 1.01,
        "click: {}",
        max_step(&l)
    );
}

#[test]
fn automation_of_the_switch_is_the_base_of_its_modulation() {
    let mut p = create(config());
    let (mut t, rack, b) = dc_track(&mut p);
    // The switch's automation turns it off at beat 1; the macro (0.7, depth +1) holds it on
    // regardless; at beat 2 the macro drops to 0.3 and the automated base (off) shows.
    t.modulation = ModulationDesc {
        modulators: vec![],
        mappings: vec![switch_mapping(
            ModSourceDesc::Macro { rack, index: 0 },
            b,
            1.0,
            true,
        )],
        macros: macros(rack, 0.7),
    };
    t.automation = vec![
        steps(
            ResolvedTarget::NodeEnabled { node: b },
            SWITCH,
            &[(0.0, 1.0), (1.0, 0.0)],
        ),
        macro_lane(rack, &[(0.0, 0.7), (2.0, 0.3)]),
    ];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 3 * BEAT, ODD_BLOCK);
    assert_eq!(l[BEAT + FADE], 0.25, "the macro keeps it on");
    assert_eq!(l[2 * BEAT - 1], 0.25);
    assert!(l[2 * BEAT] > 0.25 && l[2 * BEAT] < 0.26, "{}", l[2 * BEAT]);
    assert!((l[2 * BEAT + FADE] - 0.5).abs() < 1e-6);
}

#[test]
fn an_lfo_switches_a_device_on_the_modulation_grid() {
    let mut p = create(config());
    let (mut t, _rack, b) = dc_track(&mut p);
    // Square LFO at 2 Hz (+1 for the first half period, -1 after), depth -1 from on:
    // off for 250 ms, on for 250 ms.
    let lfo = ModulatorDesc {
        id: ModulatorId(Ulid(7)),
        host: b,
        kind: ModulatorKind::Lfo,
        params: vec![(ParamId(0), 4.0), (ParamId(1), 2.0), (ParamId(2), 0.0)],
        sidechain: None,
    };
    t.modulation = ModulationDesc {
        modulators: vec![lfo],
        mappings: vec![switch_mapping(ModSourceDesc::Modulator(0), b, -1.0, true)],
        macros: vec![],
    };
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 24_000, ODD_BLOCK);
    // Off right away (fading from on), back on at the half period (12000), on the grid.
    assert!((l[FADE] - 0.5).abs() < 1e-6, "{}", l[FADE]);
    let on = (FADE..l.len()).find(|&i| l[i] < 0.5 - 1e-6).unwrap();
    assert!(
        on % 32 == 0 && on.abs_diff(12_000) <= 32,
        "switched on at {on}"
    );
    assert!((l[on + FADE] - 0.25).abs() < 1e-6);
    assert!(
        max_step(&l[1..]) <= 0.25 / FADE as f32 * 1.01,
        "click: {}",
        max_step(&l[1..])
    );
}

#[test]
fn readback_shows_the_switch_state() {
    let mut p = create(config());
    let (mut t, rack, b) = dc_track(&mut p);
    t.modulation = ModulationDesc {
        modulators: vec![],
        mappings: vec![switch_mapping(
            ModSourceDesc::Macro { rack, index: 0 },
            b,
            -1.0,
            true,
        )],
        macros: macros(rack, 0.8),
    };
    p.handle.publish(desc(1, t)).unwrap();
    render(&mut p.engine, 4800, BLOCK);
    let mut frames = Vec::new();
    p.handle.poll_analysis(|f| frames.push(*f));
    let f = frames
        .iter()
        .rfind(|f| f.kind == AnalysisKind::Modulation && f.node == b)
        .expect("a readback frame");
    let v = f.values();
    assert_eq!(v[0].to_bits(), DEVICE_ENABLED_PARAM.0);
    assert_eq!((v[1], v[2]), (1.0, 0.0), "base on, effective off");
}

/// Instrument: outputs Σ key / 128 over its held keys.
struct KeyGate {
    held: [bool; 128],
}

impl KeyGate {
    fn level(&self) -> f32 {
        (0..128)
            .filter(|&k| self.held[k])
            .map(|k| k as f32 / 128.0)
            .sum()
    }
}

impl Node for KeyGate {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.held = [false; 128];
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut pos = 0;
        for e in ctx.events {
            let off = e.offset as usize;
            let v = self.level();
            for ch in audio.outputs.iter_mut() {
                ch[pos..off].fill(v);
            }
            pos = off;
            match e.kind {
                EventKind::NoteOn { key, .. } => self.held[key as usize] = true,
                EventKind::NoteOff { key, .. } => self.held[key as usize] = false,
                EventKind::AllNotesOff => self.held = [false; 128],
                _ => {}
            }
        }
        let v = self.level();
        for ch in audio.outputs.iter_mut() {
            ch[pos..].fill(v);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// MIDI effect: transposes notes by an octave.
struct Octave;

impl Node for Octave {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        _: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            let mut e = *e;
            match &mut e.kind {
                EventKind::NoteOn { key, .. } | EventKind::NoteOff { key, .. } => *key += 12,
                EventKind::Param { .. } => continue,
                _ => {}
            }
            ctx.out_events.push(e);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 0)
    }
}

/// MIDI thru, audio through.
struct RackNode;

impl Node for RackNode {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        for e in ctx.events {
            if !matches!(e.kind, EventKind::Param { .. }) {
                ctx.out_events.push(*e);
            }
        }
        audio.pass_through();
        ProcessStatus::Continue
    }
}

#[test]
fn a_macro_switched_midi_effect_is_thru_from_the_sample_and_strands_no_note() {
    let mut p = create(config());
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let fx = p.handle.add_node(Box::new(Octave)).unwrap();
    let inst = p
        .handle
        .add_node(Box::new(KeyGate { held: [false; 128] }))
        .unwrap();
    let mut t = with_chain(
        track(tid(2), TrackKind::Midi, Some(tid(1))),
        &[rack, fx, inst],
    );
    // Key 48 held across the switch (beat 0.25..1.5), another from beat 1.25 to 1.75.
    t.clips = vec![midi_clip(
        cid(1),
        0.0,
        4.0,
        &[(0.25, 1.25, 48), (1.25, 0.5, 50)],
    )];
    t.modulation = ModulationDesc {
        modulators: vec![],
        mappings: vec![switch_mapping(
            ModSourceDesc::Macro { rack, index: 0 },
            fx,
            -1.0,
            true,
        )],
        macros: macros(rack, 0.0),
    };
    t.automation = vec![macro_lane(rack, &[(0.0, 0.0), (1.0, 1.0)])];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 2 * BEAT, ODD_BLOCK);
    let key = |k: f32| k / 128.0;
    assert_eq!(l[BEAT - 1], key(60.0), "transposed while on");
    assert_eq!(l[BEAT], 0.0, "released on the switch's sample");
    assert_eq!(l[BEAT + BEAT / 4 + 1], key(50.0), "thru while off");
    assert_eq!(l[2 * BEAT - 1], 0.0, "nothing stuck");
}

#[test]
fn a_macro_switches_a_device_on_its_rack_chain() {
    use ether_core::graph::ChainEntry;
    use ether_core::protocol::model::RackChainId;
    use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
    let mut p = create(config());
    let src = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let rack = p.handle.add_node(Box::new(RackNode)).unwrap();
    let util = p.handle.add_node(Box::new(Dc(0.25))).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, rack]);
    t.chain_racks = vec![ChainRackDesc {
        rack,
        kind: ChainRackKind::AudioEffect,
        chains: vec![RackChainDesc {
            id: RackChainId(Ulid(1)),
            chain: vec![ChainEntry {
                node: util,
                enabled: true,
                sidechain: None,
            }],
            volume: 1.0,
            pan: 0.0,
            mute: false,
            keys: (0, 127),
            velocities: (0, 127),
            select: (0, 127),
        }],
        selector: 0,
    }];
    t.modulation = ModulationDesc {
        modulators: vec![],
        mappings: vec![switch_mapping(
            ModSourceDesc::Macro { rack, index: 0 },
            util,
            -1.0,
            true,
        )],
        macros: macros(rack, 0.0),
    };
    t.automation = vec![macro_lane(rack, &[(0.0, 0.0), (1.0, 1.0)])];
    p.handle.publish(desc(1, t)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 2 * BEAT, ODD_BLOCK);
    assert_eq!(l[BEAT - 1], 0.25);
    assert!(l[BEAT] > 0.25 && l[BEAT] < 0.26, "{}", l[BEAT]);
    assert!((l[BEAT + FADE] - 0.5).abs() < 1e-6, "{}", l[BEAT + FADE]);
    assert!(
        max_step(&l) <= 0.25 / FADE as f32 * 1.01,
        "click: {}",
        max_step(&l)
    );
}
