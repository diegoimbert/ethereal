//! Measured job priorities (`crate::parallel`, native hosts): the engine measures each
//! track's job time and the handle re-ranks the job graph with it, so a track with one heavy
//! device overtakes a track of several light ones, without allocating on the audio thread.

use crate::common::*;

use std::time::{Duration, Instant};

use ether_core::protocol::model::TrackKind;
use ether_core::{
    AudioBuffers, EngineOutputs, Node, PrepareConfig, ProcessContext, ProcessStatus,
    RenderGraphDesc, TransportControl, create,
};

/// Pass-through that also burns `cost` of wall time per call.
struct Busy(Duration);

impl Node for Busy {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.pass_through();
        let t = Instant::now();
        while t.elapsed() < self.0 {
            std::hint::spin_loop();
        }
        ProcessStatus::Continue
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn measured_job_times_rerank_the_graph() {
    let mut p = create(config());
    // `light`: four near-free devices (static cost 5); `heavy`: one slow device (cost 2).
    let light: Vec<_> = (0..4)
        .map(|_| p.handle.add_node(Box::new(Busy(Duration::ZERO))).unwrap())
        .collect();
    let heavy = p
        .handle
        .add_node(Box::new(Busy(Duration::from_micros(150))))
        .unwrap();
    let (tl, th) = (tid(10), tid(11));
    let desc = RenderGraphDesc {
        version: 1,
        tracks: vec![
            master(),
            with_chain(track(tl, TrackKind::Audio, Some(tid(1))), &light),
            with_chain(track(th, TrackKind::Audio, Some(tid(1))), &[heavy]),
        ],
        ..Default::default()
    };
    p.handle.publish(desc).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let rank = |order: &[_], t| order.iter().position(|x| *x == t).unwrap();
    let block = 256;
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut out = EngineOutputs::default();
    let mut render = |p: &mut ether_core::EngineParts, seconds: f64| {
        for _ in 0..(seconds * 48_000.0 / block as f64) as usize {
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            // Measuring, reporting and swapping in a re-ranked graph allocate nothing.
            assert_no_alloc::assert_no_alloc(|| p.engine.process(&[], &mut outs, block));
            p.handle.poll(&mut out);
            p.gc.collect();
        }
    };
    render(&mut p, 0.1);
    let before = p.engine.job_order();
    assert!(rank(&before, tl) < rank(&before, th), "static: {before:?}");
    // Costs are reported every 0.5 s, the graph re-ranked at most every 2 s.
    render(&mut p, 3.0);
    let after = p.engine.job_order();
    assert!(rank(&after, th) < rank(&after, tl), "measured: {after:?}");
    // A new publish keeps the measured ranking.
    let desc = RenderGraphDesc {
        version: 2,
        tracks: vec![
            master(),
            with_chain(track(tl, TrackKind::Audio, Some(tid(1))), &light),
            with_chain(track(th, TrackKind::Audio, Some(tid(1))), &[heavy]),
        ],
        ..Default::default()
    };
    p.handle.publish(desc).unwrap();
    render(&mut p, 0.05);
    let again = p.engine.job_order();
    assert!(
        rank(&again, th) < rank(&again, tl),
        "republished: {again:?}"
    );
    assert_eq!(p.engine.leaked(), 0);
}
