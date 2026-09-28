//! Rack chains and modulation never allocate on the audio thread: every modulator kind,
//! macros, automated bases, envelope-follower sidechains, readback, instrument / audio /
//! MIDI effect racks, snapshot swaps (state carried over, unmapped params restored) and
//! transport jumps.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::graph::{
    AutomationDesc, ChainEntry, ParamMapping, RenderGraphDesc, ResolvedTarget, TrackDesc,
};
use ether_core::modulation::{
    MacroDesc, ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, Beats, CurveShape, DeviceId, ModulatorId, ModulatorKind, ParamId,
    RackChainId, TrackKind, Ulid,
};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, Engine, EventKind, Node, NodeKey, ParamChange, ParamTarget, PrepareConfig,
    ProcessContext, ProcessStatus, TransportControl, create,
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// Pass-through with MIDI-thru (rack node stand-in).
struct Thru;

impl Node for Thru {
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

const MAP: ParamMapping = ParamMapping {
    min: 0.0,
    max: 10.0,
    scale: ParamScale::Linear,
    steps: Some(11),
};

fn run(engine: &mut Engine, blocks: usize, frames: usize) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc(|| engine.process(&[], &mut outs, frames));
    }
}

fn entry(node: NodeKey) -> ChainEntry {
    ChainEntry {
        node,
        enabled: true,
        sidechain: None,
    }
}

fn chain(id: u128, nodes: &[NodeKey], keys: (u8, u8)) -> RackChainDesc {
    RackChainDesc {
        id: RackChainId(Ulid(id)),
        chain: nodes.iter().map(|&n| entry(n)).collect(),
        volume: 0.8,
        pan: -0.2,
        mute: false,
        keys,
        velocities: (0, 127),
        select: (0, 127),
    }
}

fn modulator(id: u128, host: NodeKey, kind: ModulatorKind, params: &[(u32, f64)]) -> ModulatorDesc {
    ModulatorDesc {
        id: ModulatorId(Ulid(id)),
        host,
        kind,
        params: params.iter().map(|&(p, v)| (ParamId(p), v)).collect(),
        sidechain: None,
    }
}

#[test]
fn racks_and_modulation_never_allocate() {
    let mut p = create(config());
    let rack = p.handle.add_node(Box::new(Thru)).unwrap();
    let a = p.handle.add_node(Box::new(Dc(0.3))).unwrap();
    let b = p.handle.add_node(Box::new(Dc(0.1))).unwrap();
    let fx_rack = p.handle.add_node(Box::new(Thru)).unwrap();
    let fx = p.handle.add_node(Box::new(Delay::new(64))).unwrap();
    let midi_rack = p.handle.add_node(Box::new(Thru)).unwrap();
    let midi_fx = p.handle.add_node(Box::new(Thru)).unwrap();
    let kick_src = p.handle.add_node(Box::new(Dc(0.7))).unwrap();

    let kick = {
        let t = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[kick_src]);
        t
    };
    let build = |with_mod: bool| -> TrackDesc {
        let mut t = with_chain(
            track(tid(2), TrackKind::Midi, Some(tid(1))),
            &[midi_rack, rack, fx_rack],
        );
        t.clips = vec![midi_clip(
            cid(1),
            0.0,
            8.0,
            &[(0.0, 0.5, 40), (0.5, 0.5, 70), (1.0, 2.0, 60)],
        )];
        t.chain_racks = vec![
            ChainRackDesc {
                rack: midi_rack,
                kind: ChainRackKind::MidiEffect,
                chains: vec![chain(1, &[midi_fx], (0, 127))],
                selector: 0,
            },
            ChainRackDesc {
                rack,
                kind: ChainRackKind::Instrument,
                chains: vec![chain(2, &[a], (0, 59)), chain(3, &[b], (60, 127))],
                selector: 3,
            },
            ChainRackDesc {
                rack: fx_rack,
                kind: ChainRackKind::AudioEffect,
                chains: vec![chain(4, &[fx], (0, 127)), chain(5, &[], (0, 127))],
                selector: 0,
            },
        ];
        if with_mod {
            let mut follower = modulator(15, fx_rack, ModulatorKind::EnvelopeFollower, &[]);
            follower.sidechain = Some(tid(3));
            let mods = vec![
                modulator(
                    10,
                    rack,
                    ModulatorKind::Lfo,
                    &[(1, 5.0), (5, 1.0), (6, 100.0)],
                ),
                modulator(
                    11,
                    rack,
                    ModulatorKind::Envelope,
                    &[(0, 10.0), (1, 50.0), (2, 50.0), (3, 80.0)],
                ),
                modulator(
                    12,
                    fx_rack,
                    ModulatorKind::EnvelopeFollower,
                    &[(0, 1.0), (1, 50.0)],
                ),
                modulator(
                    13,
                    rack,
                    ModulatorKind::Steps,
                    &[(0, 8.0), (1, 4.0), (2, 30.0), (4, 1.0)],
                ),
                modulator(14, rack, ModulatorKind::Random, &[(0, 7.0), (3, 50.0)]),
                follower,
                modulator(16, rack, ModulatorKind::Keytrack, &[(0, 36.0), (1, 96.0)]),
                modulator(17, rack, ModulatorKind::Velocity, &[(0, 1.0), (1, 127.0)]),
                modulator(18, rack, ModulatorKind::Lfo, &[(2, 1.0), (3, 6.0)]),
            ];
            let mut mappings: Vec<ModMappingDesc> = (0..mods.len() as u32)
                .map(|i| ModMappingDesc {
                    source: ModSourceDesc::Modulator(i),
                    node: if i % 2 == 0 { a } else { fx },
                    param: ParamId(i % 3),
                    depth: 0.3,
                    mapping: MAP,
                    base: 0.5,
                })
                .collect();
            mappings.push(ModMappingDesc {
                source: ModSourceDesc::Macro { rack, index: 1 },
                node: rack,
                param: ParamId(8),
                depth: 1.0,
                mapping: ParamMapping {
                    min: 0.0,
                    max: 127.0,
                    scale: ParamScale::Linear,
                    steps: Some(128),
                },
                base: 0.0,
            });
            t.modulation = ModulationDesc {
                modulators: mods,
                mappings,
                macros: vec![MacroDesc {
                    rack,
                    values: [0.0, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                }],
            };
            t.automation = vec![AutomationDesc {
                target: AutomationTarget::DeviceParam {
                    device: DeviceId(Ulid(99)),
                    param: ParamId(0),
                },
                resolved: ResolvedTarget::Node {
                    node: a,
                    param: ParamId(0),
                },
                points: vec![
                    (0.0, 0.0, CurveShape::Linear),
                    (4.0, 1.0, CurveShape::Linear),
                ],
                mapping: MAP,
            }];
        }
        t
    };
    let desc = |version: u64, with_mod: bool| RenderGraphDesc {
        version,
        tracks: vec![master(), kick.clone(), build(with_mod)],
        ..Default::default()
    };
    p.handle.publish(desc(1, true)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p.engine, 4, BLOCK); // snapshot swap on the first block
    run(&mut p.engine, 80, BLOCK);
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rack,
                param: ParamId(1),
            },
            value: 0.9,
        })
        .unwrap();
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Modulator {
                modulator: ModulatorId(Ulid(10)),
                param: ParamId(1),
            },
            value: 12.0,
        })
        .unwrap();
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: a,
                param: ParamId(1),
            },
            value: 3.0,
        })
        .unwrap();
    run(&mut p.engine, 20, 100);
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(0.5),
        })
        .unwrap();
    run(&mut p.engine, 20, 37);
    // Swap: same modulation (state carried over), then without it (bases restored).
    p.handle.publish(desc(2, true)).unwrap();
    run(&mut p.engine, 10, BLOCK);
    p.handle.publish(desc(3, false)).unwrap();
    run(&mut p.engine, 10, BLOCK);
    let mut n = 0;
    p.handle.poll_analysis(|_| n += 1);
    assert!(n > 0);
}
