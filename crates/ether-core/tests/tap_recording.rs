//! `tap-recording` engine side: while recording, an armed audio track whose input is another
//! track (`TrackDesc::input_tap`) has its aligned tap captured after the track jobs, with
//! its PDC input latency, so the host can place every frame on the timeline. The audio
//! thread never allocates (also with full rings).

mod common;

use assert_no_alloc::assert_no_alloc;
use common::*;
use ether_core::graph::TrackDesc;
use ether_core::protocol::model::{BeatRange, Beats, InputTap, TrackId, TrackKind};
use ether_core::recording::{CaptureBlock, MAX_TAP_CAPTURES, RecordingIo, TapCapture};
use ether_core::{
    AudioBuffers, EngineParts, InputTapDesc, Node, PrepareConfig, ProcessContext,
    ProcessStatus, RenderGraphDesc, TransportControl, create,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Samples per beat at the default 120 bpm.
const SPB: f64 = SR as f64 / 2.0;

/// The known signal at timeline sample `p` (left; the right is `-0.5 *` it).
fn known(p: i64) -> f32 {
    (p.rem_euclid(2001) * 7919 % 2001) as f32 / 1000.0 - 1.0
}

/// Plays [`known`] at the timeline position it renders (follows loops and locates).
struct Timeline;

impl Node for Timeline {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let p0 = (ctx.transport.position * SPB).round() as i64;
        let [l, r] = audio.outputs else {
            return ProcessStatus::Continue;
        };
        for (i, (l, r)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            let s = if ctx.transport.playing {
                known(p0 + i as i64)
            } else {
                0.0
            };
            *l = s;
            *r = -0.5 * s;
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

fn tap(track: TrackId, point: InputTap) -> Option<InputTapDesc> {
    Some(InputTapDesc { track, point })
}

/// An armed audio track recording `source` at `point`, to master, not monitored.
fn consumer(id: TrackId, source: TrackId, point: InputTap) -> TrackDesc {
    let mut t = track(id, TrackKind::Audio, Some(tid(1)));
    t.input_tap = tap(source, point);
    t.armed = true;
    t
}

fn publish(p: &mut EngineParts, tracks: Vec<TrackDesc>, loop_region: Option<(f64, f64)>) {
    let (s, e) = loop_region.unwrap_or((0.0, 0.0));
    p.handle
        .publish(RenderGraphDesc {
            tracks,
            loop_enabled: loop_region.is_some(),
            loop_start: s,
            loop_end: e,
            ..Default::default()
        })
        .unwrap();
    let mut controls = vec![TransportControl::SetRecording { enabled: true }];
    if loop_region.is_some() {
        controls.push(TransportControl::SetLoop {
            enabled: true,
            region: BeatRange {
                start: Beats(s),
                end: Beats(e),
            },
        });
    }
    controls.push(TransportControl::Play);
    for c in controls {
        p.handle.transport(c).unwrap();
    }
}

/// Render `blocks` blocks allocation-free.
fn run(p: &mut EngineParts, blocks: usize) {
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc(|| p.engine.process(&[], &mut outs, BLOCK));
    }
}

struct Captured {
    block: CaptureBlock,
    taps: Vec<TapCapture>,
    samples: Vec<Vec<f32>>,
}

fn drain(io: &mut RecordingIo) -> Vec<Captured> {
    let mut out = Vec::new();
    let mut hw = Vec::new();
    while let Some(block) = io.capture.next_block(&mut hw) {
        hw.clear();
        let taps = io.capture.taps().to_vec();
        let samples = (0..taps.len())
            .map(|i| io.capture.tap_samples(i).to_vec())
            .collect();
        out.push(Captured {
            block,
            taps,
            samples,
        });
    }
    out
}

/// Place every captured frame of `track` on the timeline like the host does (frame `k` at
/// the position rendered at `k - latency`) and compare it with [`known`]; returns the
/// number of frames compared.
fn check_aligned(captured: &[Captured], track: TrackId, latency: u32) -> usize {
    let mut compared = 0;
    for c in captured {
        let Some(j) = c.taps.iter().position(|t| t.track == track) else {
            continue;
        };
        assert_eq!(c.taps[j].latency, latency, "{:?}", c.taps[j]);
        let s = &c.samples[j];
        assert_eq!(s.len(), c.block.frames as usize * 2);
        for i in 0..c.block.frames as u64 {
            let Some(h) = (c.block.sample_time + i).checked_sub(u64::from(latency)) else {
                continue;
            };
            // The block that rendered `h` (the first captured block starts at sample 0).
            let Some(src) = captured
                .iter()
                .find(|b| b.block.sample_time <= h && h < b.block.sample_time + u64::from(b.block.frames))
            else {
                continue;
            };
            let pos = src.block.position_at(h - src.block.sample_time);
            let expected = known((pos * SPB).round() as i64);
            let (l, r) = (s[i as usize * 2], s[i as usize * 2 + 1]);
            assert!(
                (l - expected).abs() < 1e-6 && (r + 0.5 * expected).abs() < 1e-6,
                "frame {} (timeline {pos}): {l}/{r} vs {expected}",
                c.block.sample_time + i
            );
            compared += 1;
        }
    }
    compared
}

#[test]
fn post_fx_and_post_fader_taps_are_recorded_aligned_by_pdc() {
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let gen_node = p.handle.add_node(Box::new(Timeline)).unwrap();
    let lat = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    // Source: the known signal through 100 samples of latency, to master.
    let src = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[gen_node, lat]);
    let fx = consumer(tid(3), tid(2), InputTap::PostFx);
    let fader = consumer(tid(4), tid(2), InputTap::PostFader);
    // Its own chain latency is after the tap: the take is the tap at its input.
    let lat2 = p.handle.add_node(Box::new(Delay::new(64))).unwrap();
    let fader = with_chain(fader, &[lat2]);
    publish(&mut p, vec![master(), src, fx, fader], None);
    run(&mut p, 40);
    let captured = drain(&mut io);
    assert_eq!(captured.len(), 40);
    assert!(captured.iter().all(|c| c.taps.len() == 2 && !c.block.taps_dropped));
    let n = 40 * BLOCK - 100;
    assert_eq!(check_aligned(&captured, tid(3), 100), n);
    assert_eq!(check_aligned(&captured, tid(4), 100), n);
}

#[test]
fn pre_fx_tap_latency_is_the_sources_input_latency() {
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let gen_node = p.handle.add_node(Box::new(Timeline)).unwrap();
    let lat = p.handle.add_node(Box::new(Delay::new(300))).unwrap();
    // Feeder (300 samples of latency) → group (its own 50 samples after the tap point).
    let feeder = with_chain(track(tid(5), TrackKind::Audio, Some(tid(2))), &[gen_node, lat]);
    let lat2 = p.handle.add_node(Box::new(Delay::new(50))).unwrap();
    let group = with_chain(track(tid(2), TrackKind::Group, Some(tid(1))), &[lat2]);
    let pre = consumer(tid(3), tid(2), InputTap::PreFx);
    let post = consumer(tid(4), tid(2), InputTap::PostFx);
    publish(&mut p, vec![master(), feeder, group, pre, post], None);
    run(&mut p, 30);
    let captured = drain(&mut io);
    assert_eq!(check_aligned(&captured, tid(3), 300), 30 * BLOCK - 300);
    assert_eq!(check_aligned(&captured, tid(4), 350), 30 * BLOCK - 350);
}

#[test]
fn loop_passes_capture_the_loop_region_each_time() {
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let gen_node = p.handle.add_node(Box::new(Timeline)).unwrap();
    let lat = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let src = with_chain(track(tid(2), TrackKind::Audio, None), &[gen_node, lat]);
    // Loop half a beat (12 000 samples): ~3 passes in 36 blocks of 512.
    publish(
        &mut p,
        vec![master(), src, consumer(tid(3), tid(2), InputTap::PostFx)],
        Some((0.0, 0.5)),
    );
    run(&mut p, 72);
    let captured = drain(&mut io);
    // Every frame (after the first 100) is aligned, across the loop wraps.
    assert_eq!(check_aligned(&captured, tid(3), 100), 72 * BLOCK - 100);
    let wraps = captured
        .windows(2)
        .filter(|w| w[1].block.position < w[0].block.position)
        .count();
    assert_eq!(wraps, 3);
}

#[test]
fn only_armed_audio_tap_tracks_while_recording() {
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let gen_node = p.handle.add_node(Box::new(Timeline)).unwrap();
    let src = with_chain(track(tid(2), TrackKind::Audio, None), &[gen_node]);
    let mut unarmed = consumer(tid(3), tid(2), InputTap::PostFx);
    unarmed.armed = false;
    let armed = consumer(tid(4), tid(2), InputTap::PostFx);
    p.handle
        .publish(RenderGraphDesc {
            tracks: vec![master(), src, unarmed, armed],
            ..Default::default()
        })
        .unwrap();
    // Playing without recording: nothing is captured.
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p, 4);
    assert!(drain(&mut io).is_empty());
    p.handle
        .transport(TransportControl::SetRecording { enabled: true })
        .unwrap();
    run(&mut p, 4);
    let captured = drain(&mut io);
    assert_eq!(captured.len(), 4);
    for c in &captured {
        assert_eq!(
            c.taps,
            vec![TapCapture {
                track: tid(4),
                latency: 0
            }]
        );
    }
    assert_eq!(check_aligned(&captured, tid(4), 0), 4 * BLOCK);
}

#[test]
fn tap_capture_never_allocates_even_with_full_rings() {
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().unwrap();
    let gen_node = p.handle.add_node(Box::new(Timeline)).unwrap();
    let src = with_chain(track(tid(2), TrackKind::Audio, None), &[gen_node]);
    let mut tracks = vec![master(), src];
    // More armed tap tracks than one block captures, plus an armed hardware input.
    for i in 0..MAX_TAP_CAPTURES as u128 + 2 {
        tracks.push(consumer(tid(10 + i), tid(2), InputTap::PostFader));
    }
    let mut hw = track(tid(3), TrackKind::Audio, Some(tid(1)));
    hw.armed = true;
    hw.audio_input = Some((0, 2));
    tracks.push(hw);
    publish(&mut p, tracks, Some((0.0, 1.0)));
    let input = [0.25f32; BLOCK];
    let inputs: [&[f32]; 2] = [&input, &input];
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    // Nobody drains: the tap ring fills up and taps are dropped, still allocation-free.
    for _ in 0..2 * SR as usize / BLOCK + 50 {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc(|| p.engine.process(&inputs, &mut outs, BLOCK));
    }
    let mut buf = Vec::new();
    let mut taps_dropped = false;
    let mut blocks = 0;
    while let Some(b) = io.capture.next_block(&mut buf) {
        assert_eq!(b.taps as usize, MAX_TAP_CAPTURES);
        assert_eq!(io.capture.taps().len(), MAX_TAP_CAPTURES);
        assert_eq!(
            io.capture.tap_buffer().len(),
            MAX_TAP_CAPTURES * b.frames as usize * 2
        );
        taps_dropped |= b.taps_dropped;
        blocks += 1;
        buf.clear();
    }
    assert!(blocks > 0);
    assert!(taps_dropped, "a full tap ring drops taps instead of blocking");
    assert_eq!(p.engine.leaked(), 0);
}
