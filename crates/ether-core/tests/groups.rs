//! `groups-buses` engine behaviour (CONTRACTS.md §12.10): VCA gain/mute/automation, track
//! input taps (points, monitoring, PDC alignment) and the solo/mute rules of groups and
//! buses. Everything on the audio thread is checked allocation-free.

mod common;

use assert_no_alloc::assert_no_alloc;
use common::*;
use ether_core::graph::{AutomationDesc, ParamMapping, ResolvedTarget, TrackDesc};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{AutomationTarget, CurveShape, InputTap, TrackId, TrackKind};
use ether_core::vca::VcaDesc;
use ether_core::{EngineParts, InputTapDesc, ParamChange, ParamTarget, RenderGraphDesc, create};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

fn vca(id: TrackId, volume: f32, parent: Option<TrackId>) -> VcaDesc {
    VcaDesc {
        id,
        volume,
        mute: false,
        parent,
        automation: vec![],
    }
}

/// A track playing a constant `dc` through its own chain, routed to `output`.
fn dc_track(p: &mut EngineParts, id: TrackId, dc: f32, output: Option<TrackId>) -> TrackDesc {
    let key = p.handle.add_node(Box::new(Dc(dc))).unwrap();
    with_chain(track(id, TrackKind::Audio, output), &[key])
}

fn publish(p: &mut EngineParts, tracks: Vec<TrackDesc>, vcas: Vec<VcaDesc>) {
    p.handle
        .publish(RenderGraphDesc {
            tracks,
            vcas,
            ..Default::default()
        })
        .unwrap();
}

/// Render `blocks` blocks allocation-free; returns the last block's left channel.
fn run(p: &mut EngineParts, blocks: usize) -> Vec<f32> {
    let mut out = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    for _ in 0..blocks {
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        assert_no_alloc(|| p.engine.process(&[], &mut outs, BLOCK));
    }
    out[0].clone()
}

fn settled(p: &mut EngineParts) -> f32 {
    *run(p, 4).last().unwrap()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn vca_gain_adds_up_the_chain_in_db() {
    let mut p = create(config());
    let mut a = dc_track(&mut p, tid(2), 1.0, Some(tid(1)));
    a.volume = 0.5;
    a.vca = Some(tid(20));
    let b = dc_track(&mut p, tid(3), 1.0, Some(tid(1)));
    // VCA 20 (x0.5) is assigned to VCA 21 (x0.5): a = 0.5 * 0.5 * 0.5; b is unassigned.
    publish(
        &mut p,
        vec![master(), a, b],
        vec![vca(tid(20), 0.5, Some(tid(21))), vca(tid(21), 0.5, None)],
    );
    let v = settled(&mut p);
    assert!(close(v, 0.125 + 1.0), "{v}");
}

#[test]
fn live_vca_fader_and_mute() {
    let mut p = create(config());
    let mut a = dc_track(&mut p, tid(2), 1.0, Some(tid(1)));
    a.vca = Some(tid(20));
    publish(&mut p, vec![master(), a], vec![vca(tid(20), 1.0, None)]);
    assert!(close(settled(&mut p), 1.0));
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::TrackVolume { track: tid(20) },
            value: 0.25,
        })
        .unwrap();
    assert!(close(settled(&mut p), 0.25));
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::TrackMute { track: tid(20) },
            value: 1.0,
        })
        .unwrap();
    assert_eq!(settled(&mut p), 0.0);
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::TrackMute { track: tid(20) },
            value: 0.0,
        })
        .unwrap();
    assert!(close(settled(&mut p), 0.25));
}

#[test]
fn muted_parent_vca_mutes_the_tracks_of_its_children() {
    let mut p = create(config());
    let mut a = dc_track(&mut p, tid(2), 1.0, Some(tid(1)));
    a.vca = Some(tid(20));
    let mut parent = vca(tid(21), 1.0, None);
    parent.mute = true;
    publish(
        &mut p,
        vec![master(), a],
        vec![vca(tid(20), 1.0, Some(tid(21))), parent],
    );
    assert_eq!(settled(&mut p), 0.0);
}

#[test]
fn vca_volume_automation_drives_the_gain() {
    let mut p = create(config());
    let mut a = dc_track(&mut p, tid(2), 1.0, Some(tid(1)));
    a.vca = Some(tid(20));
    let mut v = vca(tid(20), 1.0, None);
    v.automation = vec![AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(20) },
        resolved: ResolvedTarget::TrackVolume,
        points: vec![(0.0, 0.5, CurveShape::Linear)],
        // Linear 0..=1: the plain value is the linear gain.
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }];
    publish(&mut p, vec![master(), a], vec![v]);
    assert!(close(settled(&mut p), 0.5));
}

#[test]
fn input_tap_points_and_monitoring() {
    for (point, expect) in [
        // The source's Dc is its chain: nothing before it, 1.0 after, x0.5 after its fader.
        (InputTap::PreFx, 0.0),
        (InputTap::PostFx, 1.0),
        (InputTap::PostFader, 0.5),
    ] {
        for monitor in [false, true] {
            let mut p = create(config());
            let mut src = dc_track(&mut p, tid(2), 1.0, None);
            src.volume = 0.5;
            let mut dst = track(tid(3), TrackKind::Audio, Some(tid(1)));
            dst.input_tap = Some(InputTapDesc {
                track: tid(2),
                point,
            });
            dst.monitor = monitor;
            // Consumer listed before its source: the compiler orders the source first.
            publish(&mut p, vec![master(), dst, src], vec![]);
            let v = settled(&mut p);
            let want = if monitor { expect } else { 0.0 };
            assert!(close(v, want), "{point:?} monitor={monitor}: {v}");
        }
    }
}

#[test]
fn tapped_signal_is_aligned_with_the_rest_of_the_mix() {
    let mut p = create(config());
    // Source: impulse through a 100-sample latency device, not routed anywhere.
    let imp = p.handle.add_node(Box::new(Impulse)).unwrap();
    let lat = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let src = with_chain(track(tid(2), TrackKind::Audio, None), &[imp, lat]);
    // Consumer taps it post-FX and goes to master.
    let mut dst = track(tid(3), TrackKind::Audio, Some(tid(1)));
    dst.input_tap = Some(InputTapDesc {
        track: tid(2),
        point: InputTap::PostFx,
    });
    dst.monitor = true;
    // A dry impulse straight to master: PDC delays it by the tap's latency.
    let dry_imp = p.handle.add_node(Box::new(Impulse)).unwrap();
    let dry = with_chain(track(tid(4), TrackKind::Audio, Some(tid(1))), &[dry_imp]);
    publish(&mut p, vec![master(), src, dst, dry], vec![]);
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    let peaks: Vec<(usize, f32)> = l
        .iter()
        .enumerate()
        .filter(|(_, v)| v.abs() > 1e-6)
        .map(|(i, v)| (i, *v))
        .collect();
    assert_eq!(peaks, vec![(100, 2.0)]);
}

#[test]
fn pre_fx_tap_is_aligned_to_the_consumer() {
    let mut p = create(config());
    // Source: an impulse on its input bus (from a feeder), then 100 samples of latency.
    let imp = p.handle.add_node(Box::new(Impulse)).unwrap();
    let feeder = with_chain(track(tid(5), TrackKind::Audio, Some(tid(2))), &[imp]);
    let lat = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let src = with_chain(track(tid(2), TrackKind::Group, None), &[lat]);
    // Consumer taps it pre-FX (latency 0) and has 50 samples of its own.
    let lat2 = p.handle.add_node(Box::new(Delay::new(50))).unwrap();
    let mut dst = with_chain(track(tid(3), TrackKind::Audio, Some(tid(1))), &[lat2]);
    dst.input_tap = Some(InputTapDesc {
        track: tid(2),
        point: InputTap::PreFx,
    });
    dst.monitor = true;
    publish(&mut p, vec![master(), feeder, src, dst], vec![]);
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    let peak = l.iter().position(|v| v.abs() > 1e-6);
    assert_eq!(peak, Some(50));
}

#[test]
fn solo_passes_through_every_bus_on_the_output_path() {
    let mut p = create(config());
    // t (soloed) → bus g (explicit output, not its tree parent) → bus h → master.
    let mut t = dc_track(&mut p, tid(2), 1.0, Some(tid(10)));
    t.solo = true;
    let g = track(tid(10), TrackKind::Group, Some(tid(11)));
    let h = track(tid(11), TrackKind::Group, Some(tid(1)));
    // u is not soloed: silent.
    let u = dc_track(&mut p, tid(3), 0.25, Some(tid(1)));
    publish(&mut p, vec![master(), t, g, h, u], vec![]);
    assert!(close(settled(&mut p), 1.0));
}

#[test]
fn soloed_group_solos_its_children_and_muted_group_mutes_them() {
    let mut p = create(config());
    let mut g = track(tid(10), TrackKind::Group, Some(tid(1)));
    g.solo = true;
    let mut child = dc_track(&mut p, tid(2), 1.0, Some(tid(10)));
    child.group = Some(tid(10));
    let other = dc_track(&mut p, tid(3), 0.25, Some(tid(1)));
    publish(
        &mut p,
        vec![master(), g.clone(), child.clone(), other],
        vec![],
    );
    assert!(close(settled(&mut p), 1.0));

    g.solo = false;
    g.mute = true;
    let other = dc_track(&mut p, tid(3), 0.25, Some(tid(1)));
    publish(&mut p, vec![master(), g, child, other], vec![]);
    assert!(close(settled(&mut p), 0.25));
}

#[test]
fn snapshot_swaps_keep_taps_and_vcas_allocation_free() {
    let mut p = create(config());
    for round in 0..3 {
        let mut src = dc_track(&mut p, tid(2), 1.0, Some(tid(1)));
        src.vca = Some(tid(20));
        let mut dst = track(tid(3), TrackKind::Audio, Some(tid(1)));
        dst.input_tap = Some(InputTapDesc {
            track: tid(2),
            point: InputTap::PostFader,
        });
        dst.monitor = true;
        publish(
            &mut p,
            vec![master(), src, dst],
            vec![vca(tid(20), 0.5, None)],
        );
        // Source 0.5 to master + its post-fader tap (VCA included) 0.5 through dst.
        let v = settled(&mut p);
        assert!(close(v, 1.0), "round {round}: {v}");
    }
}
