//! `stream-host` (docs/COLLAB.md §9.1, §9.4): the native sender streams a synthetic tap to a
//! str0m listener over real UDP on 127.0.0.1. Checks the audio (decoded Opus matches the
//! tapped chirp at the RTP timestamps the anchors predict), that the wire timestamps are
//! `o + n` in the decoded timeline, and every anchor rule: periodic 100 ms cadence, play,
//! locate, loop wrap, latency change (+L48), gap, stop, count-in, at 48 kHz and through the
//! 44.1 kHz resampling path.

#[path = "stream_support.rs"]
mod stream_util;

use std::time::{Duration, Instant};

use ether_controller::streaming::StreamLinkState;
use ether_core::protocol::collab::{StreamClock, StreamSignal};
use ether_core::stream_tap::{StreamBlock, StreamTapWriter, stream_tap_ring};
use ether_native::stream::StreamTransport;
use stream_util::*;

/// The tapped test signal at engine time `t` (s): a chirp sweeping 200 → 2000 Hz every 2 s.
fn chirp(t: f64) -> f64 {
    let t = t % 2.0;
    0.4 * (std::f64::consts::TAU * (200.0 * t + 0.5 * 900.0 * t * t)).sin()
}

/// One scripted tap block.
#[derive(Clone, Copy, Debug)]
struct Fed {
    /// Stream index `n(T)` expected for it.
    n: u64,
    l48: u64,
    block: StreamBlock,
    discontinuity: bool,
}

struct Tap {
    w: StreamTapWriter,
    sr: u32,
    frames: u32,
    k: u64,
    fed: Vec<Fed>,
    was_playing: bool,
    last_latency: u32,
}

impl Tap {
    fn push(
        &mut self,
        position: f64,
        playing: bool,
        recording: bool,
        latency: u32,
        jump: bool,
        gap: bool,
    ) {
        let n = (self.k as u128 * 48_000 / self.sr as u128) as u64; // exact: k is a multiple of sr/100
        let block = StreamBlock {
            sample_time: self.k,
            frames: self.frames,
            position,
            playing,
            recording,
            bpm: 120.0,
            latency,
            jump,
            gap,
        };
        let discontinuity = jump
            || gap
            || (self.was_playing && !playing)
            || (latency != self.last_latency && !self.fed.is_empty());
        self.was_playing = playing;
        self.last_latency = latency;
        self.w.blocks.push(block).unwrap();
        for i in 0..self.frames as u64 {
            let s = chirp((self.k + i) as f64 / self.sr as f64) as f32;
            self.w.audio.push(s).unwrap();
            self.w.audio.push(0.5 * s).unwrap();
        }
        self.k += self.frames as u64;
        self.fed.push(Fed {
            n,
            l48: (latency as u64 * 48_000 + self.sr as u64 / 2) / self.sr as u64,
            block,
            discontinuity,
        });
    }
}

const LOOP: (f64, f64) = (8.0, 16.0);

fn transport(count_in_end: Option<f64>) -> StreamTransport {
    StreamTransport {
        loop_enabled: true,
        loop_start: LOOP.0,
        loop_end: LOOP.1,
        metronome: true,
        count_in_end,
    }
}

fn session(sr: u32) {
    let mut sender = loopback_sender();
    let mut out = Outputs::default();
    let (w, r) = stream_tap_ring(sr as usize);
    sender.start_capture(r, sr).unwrap();
    // The record start of the count-in below (the controller sets it for the pre-roll; an
    // anchor carries it only while playing + recording before it).
    sender.set_transport(transport(Some(0.0))).unwrap();
    let frames = sr / 100; // 10 ms blocks
    let lat = sr / 100; // L = 10 ms → L48 = 480
    let mut tap = Tap {
        w,
        sr,
        frames,
        k: 0,
        fed: Vec::new(),
        was_playing: false,
        last_latency: lat,
    };

    // Script (block index → state). 2 beats/s: 0.02 beats per 10 ms block.
    let mut pos = 0.0;
    let mut playing = true;
    let mut recording = false;
    let mut latency = lat;
    let step = |i: usize,
                tap: &mut Tap,
                pos: &mut f64,
                playing: &mut bool,
                recording: &mut bool,
                latency: &mut u32| {
        let mut jump = i == 0;
        let mut gap = false;
        match i {
            100 => (*pos, jump) = (4.0, true),    // locate
            130 => *latency = 2 * lat,            // graph latency change
            150 => (*pos, jump) = (LOOP.0, true), // loop wrap
            170 => gap = true,                    // tap gap
            190 => *playing = false,              // stop
            // Recording with a count-in: pre-roll from -0.4 to the record start 0.0.
            210 => (*pos, jump, *playing, *recording) = (-0.4, true, true, true),
            _ => {}
        }
        tap.push(*pos, *playing, *recording, *latency, jump, gap);
        if *playing {
            *pos += 0.02;
        }
    };

    // Feed the first blocks while connecting (they only advance the clock).
    let start = Instant::now();
    let mut i = 0usize;
    let mut listener = {
        let mut feed = || {
            while start + Duration::from_millis(10 * i as u64) <= Instant::now() && i < 40 {
                step(
                    i,
                    &mut tap,
                    &mut pos,
                    &mut playing,
                    &mut recording,
                    &mut latency,
                );
                i += 1;
            }
        };
        connect(&mut sender, &mut out, &mut feed)
    };
    assert!(i < 40, "connecting took too long ({i} blocks)");
    // Real-time feed to the end of the script.
    const BLOCKS: usize = 260;
    let end = start + Duration::from_millis(10 * BLOCKS as u64 + 300);
    while Instant::now() < end {
        while i < BLOCKS && start + Duration::from_millis(10 * i as u64) <= Instant::now() {
            step(
                i,
                &mut tap,
                &mut pos,
                &mut playing,
                &mut recording,
                &mut latency,
            );
            i += 1;
        }
        listener.pump(Duration::from_millis(2));
        out.collect(&mut sender);
    }
    assert_eq!(i, BLOCKS);

    // ── Anchors ──
    let clocks = out.clocks();
    assert!(clocks.len() > 20, "{} anchors", clocks.len());
    // The first anchor is for a block before the locate: position = 0.02 × block index, so
    // it tells this listener's offset `o` = rtp − n − L48.
    let first = &clocks[0];
    let fi = (first.position.0 / 0.02).round() as usize;
    assert!(fi < 100);
    let o = first
        .rtp
        .wrapping_sub((tap.fed[fi].n + tap.fed[fi].l48) as u32);

    let mut matched: Vec<(StreamClock, Fed)> = Vec::new();
    for c in &clocks {
        let m = c.rtp.wrapping_sub(o) as u64;
        let f = tap
            .fed
            .iter()
            .find(|f| f.n + f.l48 == m && (f.block.position - c.position.0).abs() < 1e-9)
            .unwrap_or_else(|| panic!("anchor {c:?} matches no tapped block (m = {m})"));
        assert_eq!(c.playing, f.block.playing, "{c:?}");
        assert_eq!(c.recording, f.block.recording, "{c:?}");
        assert_eq!(c.bpm, 120.0);
        assert!(c.loop_enabled && c.metronome);
        assert_eq!((c.loop_region.start.0, c.loop_region.end.0), LOOP);
        if f.discontinuity {
            assert!(c.discontinuity, "{c:?} should be a discontinuity");
        }
        matched.push((c.clone(), *f));
    }
    // Every scripted discontinuity after the connection has its anchor, `discontinuity`
    // set and `rtp = o + n + L48`.
    let at = |block: usize| {
        let f = tap.fed[block];
        matched
            .iter()
            .find(|(_, g)| g.n == f.n)
            .map(|(c, _)| c.clone())
            .unwrap_or_else(|| panic!("no anchor for block {block}"))
    };
    let lat48 = 480u32;
    let locate = at(100);
    assert!(locate.discontinuity && locate.playing);
    assert_eq!(locate.position.0, 4.0);
    assert_eq!(locate.rtp, o.wrapping_add(100 * 480 + lat48));
    let relatency = at(130);
    assert!(relatency.discontinuity);
    assert_eq!(
        relatency.rtp,
        o.wrapping_add(130 * 480 + 2 * lat48),
        "L48 follows L"
    );
    let wrap = at(150);
    assert!(wrap.discontinuity);
    assert_eq!(wrap.position.0, LOOP.0);
    assert_eq!(wrap.rtp, o.wrapping_add(150 * 480 + 2 * lat48));
    assert!(at(170).discontinuity, "gap");
    let stop = at(190);
    assert!(stop.discontinuity && !stop.playing);
    assert_eq!(stop.rtp, o.wrapping_add(190 * 480 + 2 * lat48));
    let count_in = at(210);
    assert!(count_in.discontinuity && count_in.playing && count_in.recording);
    assert_eq!(count_in.position.0, -0.4);
    assert_eq!(count_in.count_in_end.map(|b| b.0), Some(0.0));
    for (c, _) in &matched {
        let pre_roll = c.recording && c.playing && c.position.0 < 0.0;
        assert_eq!(c.count_in_end.is_some(), pre_roll, "{c:?}");
    }
    assert!(
        matched
            .iter()
            .any(|(c, _)| c.recording && c.position.0 >= 0.0),
        "an anchor after the count-in"
    );
    // Periodic cadence: 100 ms = 4800 stream samples after the previous anchor.
    for w in matched.windows(2) {
        let (a, b) = (&w[0].1, &w[1].1);
        if !w[1].0.discontinuity {
            assert_eq!(
                b.n - a.n,
                4800,
                "cadence between {:?} and {:?}",
                w[0].0,
                w[1].0
            );
        } else {
            assert!(b.n > a.n);
        }
    }

    // ── Media ──
    // Every packet is a 20 ms frame on one grid in the listener's RTP timeline.
    let ps = &listener.packets;
    assert!(ps.len() > 80, "{} packets", ps.len());
    let residue = ps[0].ts.wrapping_sub(o) % 960;
    assert!(ps.iter().all(|p| p.ts.wrapping_sub(o) % 960 == residue));
    // Decoded sample at RTP `o + m` is the tapped audio at stream index `m` (engine time
    // m / 48000): correlate at lag 0 and make sure no other lag fits better.
    let decoded = decode(ps, 10);
    let score = |lag: i64| -> f64 {
        let (mut xy, mut xx, mut yy) = (0.0, 0.0, 0.0);
        for (ts, l, _) in &decoded {
            for (j, &x) in l.iter().enumerate() {
                let m = ts.wrapping_sub(o) as i64 + j as i64 + lag;
                let y = chirp(m as f64 / 48_000.0);
                xy += x as f64 * y;
                xx += (x as f64).powi(2);
                yy += y * y;
            }
        }
        xy / (xx * yy).sqrt()
    };
    let at0 = score(0);
    assert!(
        at0 > 0.9,
        "decoded audio vs the tap at the anchored index: {at0}"
    );
    for lag in [-8, -4, -2, 2, 4, 8] {
        assert!(score(lag) < at0, "lag {lag} fits better than 0");
    }
    // Right channel is half the left.
    let (_, l, r) = &decoded[decoded.len() / 2];
    let (el, er): (f32, f32) = (l.iter().map(|x| x * x).sum(), r.iter().map(|x| x * x).sum());
    assert!((er / el - 0.25).abs() < 0.05, "stereo {el} {er}");

    // ── Bye → Closed ──
    sender
        .signal(LISTENER, STREAM, &StreamSignal::Bye { reason: None })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !out.states().contains(&StreamLinkState::Closed) {
        assert!(Instant::now() < deadline, "no Closed: {:?}", out.states());
        listener.pump(Duration::from_millis(2));
        out.collect(&mut sender);
    }
    assert!(
        !out.states()
            .iter()
            .any(|s| matches!(s, StreamLinkState::Failed { .. }))
    );
}

#[test]
fn streams_at_48k_with_exact_anchors() {
    session(48_000);
}

#[test]
fn streams_at_44k1_through_the_resampler() {
    session(44_100);
}

#[test]
fn unknown_streams_are_ignored() {
    let mut sender = loopback_sender();
    let mut out = Outputs::default();
    sender
        .signal(LISTENER, 1, &StreamSignal::Answer { sdp: "x".into() })
        .unwrap();
    sender.close(LISTENER, 1).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    out.collect(&mut sender);
    assert!(out.all.is_empty(), "{:?}", out.all);
}
