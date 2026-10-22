//! A suspended engine (`EngineHandle::set_suspended`, while a project loads): silent, no
//! graph work, no transport movement, everything sent meanwhile applied in order; the
//! resume starts from a clean state.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::common;

use common::*;
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::node::{Node, ProcessContext, ProcessStatus};
use ether_core::protocol::model::TrackKind;
use ether_core::{EventKind, RenderGraphDesc, TransportControl, create};

fn desc(tracks: Vec<ether_core::graph::TrackDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

/// A constant signal that counts its `process` and `reset` calls and the all-notes-off it
/// gets.
struct Counting {
    processed: Arc<AtomicU32>,
    resets: Arc<AtomicU32>,
    notes_off: Arc<AtomicU32>,
}

impl Node for Counting {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {
        self.resets.fetch_add(1, Ordering::Relaxed);
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.processed.fetch_add(1, Ordering::Relaxed);
        let off = ctx
            .events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::AllNotesOff))
            .count();
        self.notes_off.fetch_add(off as u32, Ordering::Relaxed);
        for ch in audio.outputs.iter_mut() {
            ch.fill(0.5);
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

struct Counters {
    processed: Arc<AtomicU32>,
    resets: Arc<AtomicU32>,
    notes_off: Arc<AtomicU32>,
}

fn counting() -> (Box<Counting>, Counters) {
    let c = Counters {
        processed: Arc::default(),
        resets: Arc::default(),
        notes_off: Arc::default(),
    };
    let node = Box::new(Counting {
        processed: c.processed.clone(),
        resets: c.resets.clone(),
        notes_off: c.notes_off.clone(),
    });
    (node, c)
}

fn silent(buf: &[f32]) -> bool {
    buf.iter().all(|s| *s == 0.0)
}

#[test]
fn a_suspended_engine_is_silent_and_runs_no_graph_work() {
    let mut p = create(config());
    let (node, c) = counting();
    let key = p.handle.add_node(node).unwrap();
    let t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[key]);
    p.handle.publish(desc(vec![master(), t])).unwrap();
    // Heard before the suspension.
    let (l, _) = render(&mut p.engine, 2048, BLOCK);
    assert!(!silent(&l[1024..]));

    p.handle.set_suspended(true);
    assert!(p.handle.suspended());
    let before = c.processed.load(Ordering::Relaxed);
    let start = p.handle.playhead().sample_time;
    // A play sent while suspended is applied (the state), but nothing moves.
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, 48_000, BLOCK);
    assert!(p.engine.is_suspended());
    assert!(silent(&l) && silent(&r), "suspended output is silent");
    assert_eq!(
        c.processed.load(Ordering::Relaxed),
        before,
        "no node runs while suspended"
    );
    let ph = p.handle.playhead();
    assert_eq!(ph.position.0, 0.0, "the transport does not move");
    // The sample clock keeps counting (hosts map it to wall time).
    assert_eq!(ph.sample_time, start + 48_000);
}

#[test]
fn what_is_sent_while_suspended_is_applied_and_the_resume_is_clean() {
    let mut p = create(config());
    p.handle.publish(desc(vec![master()])).unwrap();
    p.handle.set_suspended(true);
    render(&mut p.engine, 4096, BLOCK);

    // A progressive load: nodes and several partial graphs arrive while suspended.
    let (node, c) = counting();
    let key = p.handle.add_node(node).unwrap();
    for _ in 0..3 {
        p.handle.publish(desc(vec![master()])).unwrap();
        render(&mut p.engine, BLOCK, BLOCK);
    }
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[key]);
    t.volume = 0.5;
    p.handle.publish(desc(vec![master(), t])).unwrap();
    let (l, _) = render(&mut p.engine, 4096, BLOCK);
    assert!(silent(&l));
    assert_eq!(c.processed.load(Ordering::Relaxed), 0);

    p.handle.set_suspended(false);
    let (l, _) = render(&mut p.engine, BLOCK, BLOCK);
    assert!(!p.engine.is_suspended());
    // The final graph plays at once, at its level (smoothers jumped to their targets: no
    // fade in from the partial graphs' values).
    assert!((l[0] - 0.25).abs() < 1e-4, "first sample {}", l[0]);
    // The nodes start clean: reset, and all their notes released.
    assert_eq!(c.resets.load(Ordering::Relaxed), 1);
    assert_eq!(c.notes_off.load(Ordering::Relaxed), 1);
    assert_eq!(c.processed.load(Ordering::Relaxed), 1);
    // Only once: the next blocks are ordinary.
    render(&mut p.engine, 4 * BLOCK, BLOCK);
    assert_eq!(c.resets.load(Ordering::Relaxed), 1);
    assert_eq!(c.processed.load(Ordering::Relaxed), 5);
}

#[test]
fn suspended_blocks_do_not_allocate() {
    let mut p = create(config());
    let (node, _c) = counting();
    let key = p.handle.add_node(node).unwrap();
    let t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[key]);
    p.handle.publish(desc(vec![master(), t])).unwrap();
    render(&mut p.engine, BLOCK, BLOCK);
    p.handle.set_suspended(true);
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    for _ in 0..8 {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc::assert_no_alloc(|| p.engine.process(&[], &mut outs, BLOCK));
    }
    p.handle.set_suspended(false);
    let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
    assert_no_alloc::assert_no_alloc(|| p.engine.process(&[], &mut outs, BLOCK));
}
