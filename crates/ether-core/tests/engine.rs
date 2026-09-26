//! Offline engine tests: render without an audio device and check routing, mixing,
//! scheduling, transport, PDC, automation, meters and snapshot/GC plumbing.

mod common;

use common::*;
use ether_core::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ParamMapping, ResolvedTarget, compile,
    compile_with,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, BeatRange, Beats, CurveShape, MediaId, ParamId, TempoCurve, TrackKind, Ulid,
};
use ether_core::tempo::TempoPointDesc;
use ether_core::{
    CompileError, EngineOutputs, EventKind, NodeInfo, ParamChange, ParamTarget, RenderGraphDesc,
    TransportControl, create,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn desc(tracks: Vec<ether_core::graph::TrackDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

#[test]
fn dc_through_master_with_volume_and_pan() {
    let mut p = create(config());
    let dc = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let mut src = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[dc]);
    src.volume = 0.5;
    p.handle.publish(desc(vec![master(), src])).unwrap();
    let (l, r) = render(&mut p.engine, 2048, BLOCK);
    assert!(
        close(l[2000], 0.25) && close(r[2000], 0.25),
        "{} {}",
        l[2000],
        r[2000]
    );

    // Live pan hard right (smoothed), then check.
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::TrackPan { track: tid(2) },
            value: 1.0,
        })
        .unwrap();
    let (l, r) = render(&mut p.engine, 2048, BLOCK);
    assert!(close(l[2000], 0.0) && close(r[2000], 0.25));
    // The ramp is smooth (no jump at the first sample).
    assert!(l[0] > 0.24);
}

#[test]
fn sample_accurate_notes_across_blocks() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = vec![midi_clip(
        cid(1),
        0.0,
        8.0,
        &[(1.0, 0.5, 60), (1.0 + 1.0 / 3.0, 0.25, 62)],
    )];
    p.handle.publish(desc(vec![master(), t])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 48_000, 500);
    let ev = events(&drain(&mut rx));
    // 120 BPM @ 48 kHz: 24000 samples per beat.
    let on: Vec<_> = ev
        .iter()
        .filter(|(_, k)| matches!(k, EventKind::NoteOn { .. }))
        .map(|(t, _)| *t)
        .collect();
    let off: Vec<_> = ev
        .iter()
        .filter(|(_, k)| matches!(k, EventKind::NoteOff { .. }))
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(on, vec![24_000, 32_000]);
    assert_eq!(off, vec![36_000, 38_000]);
}

#[test]
fn loop_wraps_and_releases_notes() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = vec![midi_clip(cid(1), 0.0, 8.0, &[(1.5, 2.0, 60)])];
    let mut d = desc(vec![master(), t]);
    d.loop_enabled = true;
    d.loop_start = 0.0;
    d.loop_end = 2.0;
    p.handle.publish(d).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 110_000, 512);
    let seen = drain(&mut rx);
    let ev = events(&seen);
    let simple: Vec<_> = ev
        .iter()
        .filter_map(|(t, k)| match k {
            EventKind::NoteOn { .. } => Some((*t, true)),
            EventKind::NoteOff { .. } => Some((*t, false)),
            _ => None,
        })
        .collect();
    assert_eq!(
        simple,
        vec![
            (36_000, true),
            (48_000, false),
            (84_000, true),
            (96_000, false)
        ]
    );
    // The block containing the loop end is split exactly there and position wraps.
    assert!(
        seen.iter()
            .any(|s| matches!(s, Seen::Block(48_000, _, pos, _, _) if pos.abs() < 1e-9))
    );
    let ph = p.handle.playhead();
    assert!(ph.playing && ph.position.0 < 2.0);
}

#[test]
fn tempo_boundaries_split_blocks() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    let mut d = desc(vec![master(), t]);
    d.tempo = vec![
        TempoPointDesc {
            beat: 0.0,
            bpm: 120.0,
            curve: TempoCurve::Step,
        },
        TempoPointDesc {
            beat: 1.0,
            bpm: 60.0,
            curve: TempoCurve::Linear,
        },
        TempoPointDesc {
            beat: 3.0,
            bpm: 120.0,
            curve: TempoCurve::Step,
        },
    ];
    p.handle.publish(d).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 200_000, 1000);
    let blocks: Vec<_> = drain(&mut rx)
        .into_iter()
        .filter_map(|s| match s {
            Seen::Block(t, n, pos, bpm, bps) => Some((t, n, pos, bpm, bps)),
            _ => None,
        })
        .collect();
    // Beat 1 is at 24000 samples: a sub-block starts there at 60 BPM.
    let b = blocks
        .iter()
        .find(|b| b.0 == 24_000)
        .expect("split at beat 1");
    assert!((b.2 - 1.0).abs() < 1e-9 && (b.3 - 60.0).abs() < 1e-9);
    // Sub-blocks are contiguous and never exceed the host block.
    for w in blocks.windows(2) {
        assert_eq!(w[0].0 + w[0].1 as u64, w[1].0);
        assert!(w[0].1 <= 1000);
    }
    // Beat 3 (end of the 60->120 ramp): 0.5 s + 4·ln2/... check via position continuity.
    let ramp_end = blocks
        .iter()
        .find(|b| (b.2 - 3.0).abs() < 1e-6)
        .expect("split at beat 3");
    let expected = 0.5 + 60.0 / 30.0 * 2f64.ln(); // seconds at beat 3
    assert!((ramp_end.0 as f64 / 48_000.0 - expected).abs() < 1.0 / 48_000.0 + 1e-9);
}

#[test]
fn pdc_aligns_parallel_paths_and_sends() {
    let mut p = create(config());
    let imp_a = p.handle.add_node(Box::new(Impulse)).unwrap();
    let delay = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let imp_b = p.handle.add_node(Box::new(Impulse)).unwrap();
    let a = with_chain(
        track(tid(2), TrackKind::Midi, Some(tid(1))),
        &[imp_a, delay],
    );
    let mut b = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[imp_b]);
    b.sends = vec![send(sid(1), tid(4), 1.0, false)];
    let ret = track(tid(4), TrackKind::Return, Some(tid(1)));
    let d = desc(vec![master(), a, b, ret]);

    // Compile-level checks.
    let snap = compile_with(d.clone(), &config(), &|k| {
        Some(NodeInfo {
            latency: if k == delay { 100 } else { 0 },
            channels: (2, 2),
        })
    })
    .unwrap();
    assert_eq!(snap.track_latency(tid(2)), Some(100));
    assert_eq!(snap.output_compensation(tid(2)), Some(0));
    assert_eq!(snap.output_compensation(tid(3)), Some(100));
    assert_eq!(snap.send_compensation(sid(1)), Some(0));
    assert_eq!(snap.output_compensation(tid(4)), Some(100));
    assert_eq!(snap.latency(), 100);
    let order: Vec<_> = snap.processing_order().collect();
    let pos = |t| order.iter().position(|&x| x == t).unwrap();
    assert!(pos(tid(3)) < pos(tid(4)) && pos(tid(4)) < pos(tid(1)));

    p.handle.publish(d).unwrap();
    let (l, _) = render(&mut p.engine, 1024, 256);
    // All three paths (A delayed by its plugin, B and B's return compensated) line up.
    assert!(close(l[100], 3.0), "{}", l[100]);
    let others: f32 = l
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 100)
        .map(|(_, v)| v.abs())
        .sum();
    assert!(others < 1e-6);
}

#[test]
fn bypassed_latency_is_still_compensated() {
    let mut p = create(config());
    let imp = p.handle.add_node(Box::new(Impulse)).unwrap();
    let delay = p.handle.add_node(Box::new(Delay::new(64))).unwrap();
    let mut a = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[imp, delay]);
    a.chain[1].enabled = false;
    p.handle.publish(desc(vec![master(), a])).unwrap();
    let (l, _) = render(&mut p.engine, 512, 512);
    assert!(close(l[64], 1.0));
}

#[test]
fn mute_solo_and_group_propagation() {
    let mut p = create(config());
    let dc_c = p.handle.add_node(Box::new(Dc(1.0))).unwrap();
    let dc_d = p.handle.add_node(Box::new(Dc(0.25))).unwrap();
    let group = track(tid(10), TrackKind::Group, Some(tid(1)));
    let mut child = with_chain(track(tid(11), TrackKind::Audio, Some(tid(10))), &[dc_c]);
    child.group = Some(tid(10));
    child.sends = vec![send(sid(1), tid(20), 0.5, true)];
    let other = with_chain(track(tid(12), TrackKind::Audio, Some(tid(1))), &[dc_d]);
    let ret = track(tid(20), TrackKind::Return, Some(tid(1)));
    let base = desc(vec![master(), group, child, other, ret]);

    let settle = |p: &mut ether_core::EngineParts| render(&mut p.engine, 4096, 512).0[4000];

    p.handle.publish(base.clone()).unwrap();
    assert!(close(settle(&mut p), 1.0 + 0.25 + 0.5));

    // Solo the child: `other` goes silent, the group (its path) and returns stay.
    let mut d = base.clone();
    d.tracks[2].solo = true;
    p.handle.publish(d).unwrap();
    assert!(close(settle(&mut p), 1.0 + 0.5));

    // Solo the group: the child inside it stays audible.
    let mut d = base.clone();
    d.tracks[1].solo = true;
    p.handle.publish(d).unwrap();
    assert!(close(settle(&mut p), 1.0 + 0.5));

    // Live mute of the group propagates to the child, including its pre-fader send.
    p.handle.publish(base.clone()).unwrap();
    settle(&mut p);
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::TrackMute { track: tid(10) },
            value: 1.0,
        })
        .unwrap();
    assert!(close(settle(&mut p), 0.25));
}

fn linear_mapping() -> ParamMapping {
    ParamMapping {
        min: 0.0,
        max: 1.0,
        scale: ParamScale::Linear,
        steps: None,
    }
}

#[test]
fn volume_automation_and_fader_law() {
    let mut p = create(config());
    let dc = p.handle.add_node(Box::new(Dc(1.0))).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[dc]);
    t.automation = vec![AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved: ResolvedTarget::TrackVolume,
        points: vec![
            (0.0, 0.0, CurveShape::Linear),
            (4.0, 1.0, CurveShape::Linear),
        ],
        mapping: linear_mapping(),
    }];
    p.handle.publish(desc(vec![master(), t.clone()])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 48_000, 256);
    // At beat 2 (sample 48000) the gain is ~0.5 (block resolution + 10 ms smoothing).
    assert!((l[47_999] - 0.5).abs() < 0.02, "{}", l[47_999]);

    // Fader-law dB mapping: normalized 1.0 of (-70..+6 dB) is +6 dB.
    t.automation[0].points = vec![(0.0, 1.0, CurveShape::Step)];
    t.automation[0].mapping = ParamMapping {
        min: -70.0,
        max: 6.0,
        scale: ParamScale::Fader,
        steps: None,
    };
    p.handle.publish(desc(vec![master(), t])).unwrap();
    let (l, _) = render(&mut p.engine, 4096, 256);
    assert!((l[4000] - 10f32.powf(6.0 / 20.0)).abs() < 1e-3);
}

#[test]
fn node_param_automation_and_live_params() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.automation = vec![AutomationDesc {
        target: AutomationTarget::TrackPan { track: tid(2) },
        resolved: ResolvedTarget::Node {
            node: rec,
            param: ParamId(7),
        },
        points: vec![
            (0.0, 0.0, CurveShape::Linear),
            (1.0, 1.0, CurveShape::Linear),
        ],
        mapping: ParamMapping {
            min: 100.0,
            max: 200.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }];
    p.handle.publish(desc(vec![master(), t])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 24_000, 512);
    let ev = events(&drain(&mut rx));
    let params: Vec<_> = ev
        .iter()
        .filter_map(|(t, k)| match k {
            EventKind::Param { param, value } if *param == ParamId(7) => Some((*t, *value)),
            _ => None,
        })
        .collect();
    assert!(
        params.len() > 300,
        "dense automation events: {}",
        params.len()
    );
    assert_eq!(params[0], (0, 100.0));
    for w in params.windows(2) {
        assert!(w[1].1 > w[0].1 && w[1].0 > w[0].0);
    }
    let (t_mid, v_mid) = params[params.len() / 2];
    assert!((v_mid - (100.0 + 100.0 * t_mid as f64 / 24_000.0)).abs() < 0.5);

    // A live param change reaches the node at the start of the next block.
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rec,
                param: ParamId(3),
            },
            value: 0.25,
        })
        .unwrap();
    render(&mut p.engine, 512, 512);
    let ev = events(&drain(&mut rx));
    assert!(ev.contains(&(
        24_000,
        EventKind::Param {
            param: ParamId(3),
            value: 0.25
        }
    )));
}

#[test]
fn transport_stop_locate_release_notes() {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = vec![midi_clip(cid(1), 0.0, 8.0, &[(0.0, 4.0, 60)])];
    p.handle.publish(desc(vec![master(), t])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 1024, 512);
    p.handle.transport(TransportControl::Stop).unwrap();
    render(&mut p.engine, 1024, 512);
    let ev = events(&drain(&mut rx));
    assert!(
        ev.iter()
            .any(|e| matches!(e, (1024, EventKind::NoteOff { key: 60, .. })))
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, (1024, EventKind::AllNotesOff)))
    );
    let ph = p.handle.playhead();
    assert!(!ph.playing);
    let pos_after_stop = ph.position.0;
    assert!((pos_after_stop - 1024.0 / 24_000.0).abs() < 1e-9);

    p.handle
        .transport(TransportControl::Locate {
            position: Beats(4.0),
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 512, 512);
    assert!((p.handle.playhead().position.0 - (4.0 + 512.0 / 24_000.0)).abs() < 1e-9);

    // Loop override without republish.
    p.handle
        .transport(TransportControl::SetLoop {
            enabled: true,
            region: BeatRange {
                start: Beats(4.0),
                end: Beats(4.5),
            },
        })
        .unwrap();
    render(&mut p.engine, 24_000, 512);
    let pos = p.handle.playhead().position.0;
    assert!((4.0..4.5).contains(&pos), "{pos}");
}

#[test]
fn audio_clip_plays_source() {
    let mut p = create(config());
    let media = MediaId(Ulid(99));
    p.handle.add_source(media, mem_source(100_000)).unwrap();
    let mut t = track(tid(2), TrackKind::Audio, Some(tid(1)));
    t.clips = vec![ether_core::graph::ClipDesc {
        id: cid(1),
        start: 1.0,
        length: 2.0,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Audio {
            media,
            gain: 0.5,
            transpose: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            warp: None,
        },
        envelopes: vec![],
    }];
    p.handle.publish(desc(vec![master(), t])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, 96_000, 480);
    assert_eq!(l[23_999], 0.0);
    // After the anti-click fade the clip plays the source 1:1 (unwarped at 120 BPM).
    for i in [200usize, 777, 30_000] {
        let expected = 0.5 * ((i % 1000) as f32 / 1000.0);
        assert!(close(l[24_000 + i], expected), "{i}: {}", l[24_000 + i]);
        assert!(close(r[24_000 + i], expected));
    }
    // Clip ends at beat 3 = sample 72000.
    assert_eq!(l[72_001], 0.0);
}

#[test]
fn meters_and_polling() {
    let mut p = create(config());
    let dc = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[dc]);
    p.handle.publish(desc(vec![master(), t])).unwrap();
    render(&mut p.engine, 4800, 480);
    let mut out = EngineOutputs::default();
    p.handle.poll(&mut out);
    let m = out
        .meters
        .iter()
        .find(|m| m.track == tid(2))
        .expect("meter");
    assert!(close(m.peak[0], 0.5) && close(m.rms[1], 0.5) && !m.clipped);
    assert!(out.playhead.is_some());
}

#[test]
fn snapshots_and_nodes_are_dropped_by_gc() {
    let mut p = create(config());
    let dc = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[dc]);
    p.handle.publish(desc(vec![master(), t.clone()])).unwrap();
    p.handle.publish(desc(vec![master(), t])).unwrap();
    render(&mut p.engine, 512, 512);
    // The initial empty snapshot + the first published one.
    assert_eq!(p.gc.collect(), 2);
    p.handle.publish(desc(vec![master()])).unwrap();
    p.handle.remove_node(dc).unwrap();
    assert_eq!(
        p.handle.remove_node(dc),
        Err(ether_core::EngineError::UnknownNode(dc))
    );
    render(&mut p.engine, 512, 512);
    assert_eq!(p.gc.collect(), 2);
    // A stale key no longer compiles.
    let t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[dc]);
    assert_eq!(
        p.handle.publish(desc(vec![master(), t])),
        Err(ether_core::EngineError::Compile(CompileError::UnknownNode(
            dc
        )))
    );
    assert_eq!(p.engine.leaked(), 0);
}

#[test]
fn compile_rejects_cycles_and_unknown_tracks() {
    let cfg = config();
    let a = track(tid(2), TrackKind::Group, Some(tid(3)));
    let b = track(tid(3), TrackKind::Group, Some(tid(2)));
    assert!(matches!(
        compile(desc(vec![master(), a.clone(), b]), &cfg),
        Err(CompileError::Cycle(_))
    ));
    let mut r = track(tid(4), TrackKind::Return, Some(tid(1)));
    r.sends = vec![send(sid(1), tid(4), 1.0, false)];
    assert!(matches!(
        compile(desc(vec![master(), r]), &cfg),
        Err(CompileError::Cycle(_))
    ));
    // Send loop through a return.
    let mut x = track(tid(5), TrackKind::Return, Some(tid(1)));
    x.sends = vec![send(sid(2), tid(6), 1.0, false)];
    let mut y = track(tid(6), TrackKind::Return, Some(tid(1)));
    y.sends = vec![send(sid(3), tid(5), 1.0, false)];
    assert!(matches!(
        compile(desc(vec![master(), x, y]), &cfg),
        Err(CompileError::Cycle(_))
    ));
    let lone = track(tid(7), TrackKind::Audio, Some(tid(42)));
    assert_eq!(
        compile(desc(vec![master(), lone]), &cfg).err(),
        Some(CompileError::UnknownTrack(tid(42)))
    );
    // Duplicate node use is rejected.
    let key = ether_core::NodeKey {
        index: 0,
        generation: 1,
    };
    let mut t1 = track(tid(8), TrackKind::Audio, Some(tid(1)));
    t1.chain = vec![ChainEntry {
        node: key,
        enabled: true,
    }];
    let mut t2 = t1.clone();
    t2.id = tid(9);
    assert!(matches!(
        compile(desc(vec![master(), t1, t2]), &cfg),
        Err(CompileError::Capacity(_))
    ));
}
