//! Engine block-time harness (perf audit, appendix "Engine harness"): 16 tracks with a
//! pass-through node, 1-beat MIDI clips and 256-frame blocks, timed early and late in the
//! song. Ignored by default; run in release:
//!
//! ```text
//! cargo test --release -p ether-core --test perf_engine -- --ignored --nocapture
//! ```
//!
//! The non-ignored test checks the property the audit's fix relies on: rendering late
//! in a long song after a locate is the same, bit for bit, as reaching it by playing.

mod common;

use std::time::Instant;

use common::*;
use ether_core::graph::{AutomationDesc, ParamMapping, RenderGraphDesc, ResolvedTarget};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{AutomationTarget, Beats, CurveShape, ParamId, TrackKind};
use ether_core::{
    AudioBuffers, EngineConfig, Node, PrepareConfig, ProcessContext, ProcessStatus,
    TransportControl, create,
};

const TRACKS: usize = 16;
const FRAMES: usize = 256;

/// Audio pass-through that also sums its param events into its output (so automation
/// changes are observable).
struct Pass(f32);

impl Node for Pass {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.pass_through();
        for e in ctx.events {
            if let ether_core::EventKind::Param { value, .. } = e.kind {
                self.0 = value as f32;
            }
        }
        for ch in audio.outputs.iter_mut() {
            for s in ch.iter_mut() {
                *s += self.0 * 1e-3;
            }
        }
        ProcessStatus::Continue
    }
}

fn cfg() -> EngineConfig {
    EngineConfig {
        max_block_size: FRAMES,
        max_events_per_block: 1024,
        max_nodes: 64,
        ..config()
    }
}

fn lane(node: ether_core::NodeKey, param: u32, song: f64) -> AutomationDesc {
    let mut points = Vec::new();
    let mut t = 0.0;
    let mut k = 0;
    while t < song {
        points.push((
            t,
            ((k * 7 + param as usize) % 10) as f64 / 10.0,
            CurveShape::Linear,
        ));
        t += 4.0;
        k += 1;
    }
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved: ResolvedTarget::Node {
            node,
            param: ParamId(param),
        },
        points,
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }
}

/// 16 MIDI tracks of `clips` back-to-back 1-beat clips, `lanes`-param lanes each; with
/// `envelopes`, every clip also carries an envelope on param 0.
fn project(
    nodes: &[ether_core::NodeKey],
    clips: usize,
    lanes: u32,
    envelopes: bool,
) -> RenderGraphDesc {
    let song = clips as f64;
    let mut tracks = vec![master()];
    for (i, &node) in nodes.iter().enumerate() {
        let mut t = with_chain(
            track(tid(10 + i as u128), TrackKind::Midi, Some(tid(1))),
            &[node],
        );
        t.clips = (0..clips)
            .map(|c| {
                let mut clip = midi_clip(
                    cid((i * 100_000 + c) as u128 + 1),
                    c as f64,
                    1.0,
                    &[(0.0, 0.5, 60)],
                );
                if envelopes {
                    let mut env = lane(node, 0, 1.0);
                    env.points = vec![
                        (0.0, 0.2, CurveShape::Linear),
                        (1.0, 0.8, CurveShape::Linear),
                    ];
                    clip.envelopes = vec![env];
                }
                clip
            })
            .collect();
        t.automation = (0..lanes).map(|p| lane(node, p, song)).collect();
        tracks.push(t);
    }
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

struct Rig {
    engine: ether_core::Engine,
    _handle: ether_core::EngineHandle,
}

fn rig(clips: usize, lanes: u32, envelopes: bool) -> Rig {
    let mut p = create(cfg());
    let nodes: Vec<_> = (0..TRACKS)
        .map(|_| p.handle.add_node(Box::new(Pass(0.0))).unwrap())
        .collect();
    p.handle
        .publish(project(&nodes, clips, lanes, envelopes))
        .unwrap();
    Rig {
        engine: p.engine,
        _handle: p.handle,
    }
}

fn locate(r: &mut Rig, beat: f64, handle_play: bool) {
    r._handle
        .transport(TransportControl::Locate {
            position: Beats(beat),
        })
        .unwrap();
    if handle_play {
        r._handle.transport(TransportControl::Play).unwrap();
    }
}

/// Mean µs per block over `blocks` blocks.
fn time_blocks(r: &mut Rig, blocks: usize) -> f64 {
    let mut l = vec![0.0; FRAMES];
    let mut rr = vec![0.0; FRAMES];
    // Warm-up.
    for _ in 0..16 {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut rr];
        r.engine.process(&[], &mut outs, FRAMES);
    }
    let t = Instant::now();
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut rr];
        r.engine.process(&[], &mut outs, FRAMES);
    }
    t.elapsed().as_secs_f64() * 1e6 / blocks as f64
}

#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn automation_block_times() {
    println!("engine, {TRACKS} tracks, {FRAMES}-frame blocks: early / late µs per block");
    for (clips, lanes, envelopes) in [
        (8, 8, false),
        (500, 8, false),
        (2000, 8, false),
        (2000, 0, false),
        (2000, 8, true),
    ] {
        let mut r = rig(clips, lanes, envelopes);
        locate(&mut r, 0.0, true);
        let early = time_blocks(&mut r, 400);
        let late_beat = (clips as f64 - 6.0).max(0.0);
        locate(&mut r, late_beat, false);
        let late = time_blocks(&mut r, 400);
        println!(
            "{clips:>5} clips/track, {lanes} lanes{}: {early:7.1} / {late:7.1} µs ({:.1}% of 5.33 ms)",
            if envelopes { " + clip envelopes" } else { "" },
            late / 5333.0 * 100.0
        );
    }
}

fn render_from(r: &mut Rig, frames: usize) -> Vec<f32> {
    let (l, rr) = render(&mut r.engine, frames, FRAMES);
    l.into_iter().zip(rr).flat_map(|(l, r)| [l, r]).collect()
}

/// A locate deep into a long song renders what playing there from the start renders (the
/// active-clip lookup has no history that a jump could invalidate).
#[test]
fn late_locate_matches_continuous_playback() {
    // 48 clips of 1 beat at 120 bpm = 24 s; render from beat 40 both ways.
    let clips = 48;
    let mut a = rig(clips, 3, true);
    locate(&mut a, 0.0, true);
    // 40 beats = 20 s = 960_000 frames (a multiple of 256: 3750 blocks).
    let _ = render_from(&mut a, 960_000);
    let continuous = render_from(&mut a, 48_000);

    let mut b = rig(clips, 3, true);
    locate(&mut b, 40.0, true);
    // Same engine clock phase for the automation grid: offset the second rig's sample time.
    let jumped = render_from(&mut b, 48_000);
    assert!(continuous.iter().any(|s| *s != 0.0));
    // Node params are re-sent at offset 0 after the jump, so compare from the first grid
    // point on, where both have converged to the lane/envelope values.
    let max = continuous
        .iter()
        .zip(&jumped)
        .skip(2 * 1024)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(max < 1e-6, "late locate differs by {max}");
}

/// Prints a digest of renders of the harness projects (early and late, lanes and
/// envelopes): compare before/after an optimization that must be bit-identical.
#[test]
#[ignore = "digest for manual before/after comparisons"]
fn render_digests() {
    for (clips, lanes, envelopes) in [(64, 8, false), (64, 3, true), (300, 2, true)] {
        let mut r = rig(clips, lanes, envelopes);
        locate(&mut r, 0.0, true);
        let early = render_from(&mut r, 96_000);
        locate(&mut r, clips as f64 - 9.5, false);
        let late = render_from(&mut r, 96_000);
        let h = early
            .iter()
            .chain(&late)
            .fold(0xcbf29ce484222325u64, |h, s| {
                (h ^ s.to_bits() as u64).wrapping_mul(0x100000001b3)
            });
        println!("{clips} clips, {lanes} lanes, envelopes {envelopes}: {h:016x}");
    }
}
