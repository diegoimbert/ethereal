//! Audio clip playback cost per block (`sched::render_audio`): many tracks each playing one
//! long clip, unwarped, with fades, and Repitch-warped. A harness for before/after numbers:
//!
//! ```text
//! cargo test --release -p ether-core --test it perf_audio_clips:: -- --ignored --nocapture
//! ```

use crate::common::*;

use std::sync::Arc;
use std::time::Instant;

use ether_core::graph::{ClipContentDesc, ClipDesc, WarpDesc};
use ether_core::protocol::model::{MediaId, TrackKind, Ulid, WarpMode};
use ether_core::{AudioSource, RenderGraphDesc, TransportControl, create};

const MEDIA: MediaId = MediaId(Ulid(42));
const TRACKS: u128 = 32;
const FRAMES: usize = 256;

struct Stereo(Vec<f32>, Vec<f32>);

impl AudioSource for Stereo {
    fn channels(&self) -> u16 {
        2
    }
    fn frames(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let src = if channel == 0 { &self.0 } else { &self.1 };
        let s = (start as usize).min(src.len());
        let n = out.len().min(src.len() - s);
        out[..n].copy_from_slice(&src[s..s + n]);
        out[n..].fill(0.0);
        true
    }
}

fn clip(fades: f64, warp: Option<WarpDesc>) -> ClipDesc {
    ClipDesc {
        id: cid(1),
        start: 0.0,
        length: 64.0,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Audio {
            media: MEDIA,
            gain: 0.8,
            transpose: 0.0,
            fade_in: fades,
            fade_out: fades,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp,
            gain_envelope: None,
            transpose_envelope: None,
        },
        envelopes: vec![],
    }
}

/// Mean µs per `FRAMES`-frame block over 4 s of playback (after 0.5 s).
fn time(c: &ClipDesc) -> f64 {
    let mut p = create(config());
    // The native host's stretchers (Complex: Signalsmith; Complex+), or the web's Rust port
    // of Complex with `ETHER_PERF_RUST_COMPLEX=1`.
    if std::env::var("ETHER_PERF_RUST_COMPLEX").is_ok() {
        p.handle
            .set_stretcher_factory(Arc::new(ether_stretch::RustStretchFactory {
                preset: ether_stretch::RustPreset::Web,
            }));
    } else {
        p.handle
            .set_stretcher_factory(Arc::new(ether_stretch::SignalsmithFactory::default()));
    }
    p.handle
        .set_plus_stretcher_factory(Arc::new(ether_stretch::PlusStretchFactory));
    let n = 48_000 * 40;
    let l: Vec<f32> = (0..n).map(|i| ((i % 977) as f32 / 977.0) - 0.5).collect();
    let r = l.iter().map(|v| -v).collect();
    p.handle
        .add_source(MEDIA, Arc::new(Stereo(l, r)) as Arc<dyn AudioSource>)
        .unwrap();
    let mut tracks = vec![master()];
    for i in 0..TRACKS {
        let mut t = track(tid(10 + i), TrackKind::Audio, Some(tid(1)));
        let mut c = c.clone();
        c.id = cid(100 + i);
        t.clips = vec![c];
        tracks.push(t);
    }
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks,
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (mut bl, mut br) = (vec![0.0; FRAMES], vec![0.0; FRAMES]);
    let blocks = 48_000 * 4 / FRAMES;
    let warm = 48_000 / 2 / FRAMES;
    let mut total = 0.0;
    for b in 0..warm + blocks {
        let t = Instant::now();
        {
            let mut outs: [&mut [f32]; 2] = [&mut bl, &mut br];
            p.engine.process(&[], &mut outs, FRAMES);
        }
        if b >= warm {
            total += t.elapsed().as_secs_f64() * 1e6;
        }
        p.gc.collect();
    }
    total / blocks as f64
}

#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn audio_clip_block_times() {
    let warp = WarpDesc {
        mode: WarpMode::Repitch,
        markers: vec![(0.0, 0.0), (16.0, 7.9), (64.0, 33.0)],
        formants: Default::default(),
    };
    let stretched = |mode| WarpDesc {
        mode,
        ..warp.clone()
    };
    for (name, c) in [
        ("unwarped", clip(0.0, None)),
        ("unwarped, 1-beat fades", clip(1.0, None)),
        ("repitch-warped", clip(0.0, Some(warp.clone()))),
        (
            "complex-warped",
            clip(0.0, Some(stretched(WarpMode::Complex))),
        ),
        (
            "complex+-warped",
            clip(0.0, Some(stretched(WarpMode::ComplexPlus))),
        ),
    ] {
        let best = (0..5).map(|_| time(&c)).fold(f64::INFINITY, f64::min);
        println!(
            "audio clips ({TRACKS} tracks, {name}): {best:.1} µs per {FRAMES}-frame block, {:.1} ns per clip sample",
            best * 1e3 / (TRACKS as f64 * FRAMES as f64)
        );
    }
}
