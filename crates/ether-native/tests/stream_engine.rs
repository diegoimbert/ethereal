//! `stream-host` end to end with the real engine (no device: the test renders it): the
//! bridge-facing `StreamHost` installs the engine's stream tap, a looping transport with the
//! metronome on is streamed to a str0m listener over UDP on 127.0.0.1, and every click the
//! listener decodes sits where the anchors say its beat is heard (docs/COLLAB.md §9.4).

mod stream_util;

use std::time::{Duration, Instant};

use ether_core::graph::{MetronomeDesc, TrackDesc};
use ether_core::protocol::collab::StreamClock;
use ether_core::protocol::model::{MetronomeSound, TrackId, TrackKind, Ulid};
use ether_core::{EngineConfig, EngineParts, RenderGraphDesc, TransportControl, create};
use ether_native::stream::{SenderConfig, StreamHost};
use stream_util::*;

const SR: u32 = 48_000;
const BLOCK: usize = 480;

fn master() -> TrackDesc {
    TrackDesc {
        id: TrackId(Ulid(1)),
        kind: TrackKind::Master,
        chain: vec![],
        output: None,
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips: vec![],
        automation: vec![],
        racks: Vec::new(),
    }
}

fn render(p: &mut EngineParts) {
    let mut l = [0f32; BLOCK];
    let mut r = [0f32; BLOCK];
    let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
    p.engine.process(&[], &mut outs, BLOCK);
    p.gc.collect();
}

#[test]
fn engine_clicks_are_heard_where_the_anchors_put_their_beats() {
    let mut p = create(EngineConfig {
        sample_rate: SR,
        max_block_size: 512,
        max_nodes: 64,
        max_events_per_block: 256,
        ..EngineConfig::default()
    });
    let mut host = StreamHost::with_config(SenderConfig {
        loopback: true,
        default_route: false,
    });
    // 120 bpm, loop 0..2 beats (1 s): a wrap every second, a click every 0.5 s.
    let graph = RenderGraphDesc {
        version: 1,
        loop_enabled: true,
        loop_start: 0.0,
        loop_end: 2.0,
        metronome: true,
        click: MetronomeDesc {
            volume: 0.5,
            accent: false,
            sound: MetronomeSound::Classic,
            count_in_end: None,
        },
        tracks: vec![master()],
        ..Default::default()
    };
    host.observe_graph(&graph);
    p.handle.publish(graph).unwrap();
    host.start_capture(&mut p.handle, SR).unwrap();
    host.start_capture(&mut p.handle, SR).unwrap(); // idempotent
    render(&mut p);
    p.handle.transport(TransportControl::Play).unwrap();

    let mut out = Outputs::default();
    let start = Instant::now();
    let mut rendered = 0u64;
    let mut listener = {
        let mut tick = || {
            while start + Duration::from_micros(10_000 * rendered) <= Instant::now() {
                render(&mut p);
                rendered += 1;
            }
        };
        connect(&mut host, &mut out, &mut tick)
    };
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        while start + Duration::from_micros(10_000 * rendered) <= Instant::now() {
            render(&mut p);
            rendered += 1;
        }
        listener.pump(Duration::from_millis(2));
        out.collect(&mut host);
    }
    host.stop_capture(&mut p.handle).unwrap();
    host.stop_capture(&mut p.handle).unwrap(); // idempotent

    let clocks = out.clocks();
    assert!(clocks.iter().all(|c| c.loop_enabled && c.metronome));
    assert!(
        clocks
            .iter()
            .any(|c| c.discontinuity && c.playing && c.position.0.abs() < 1e-9),
        "a loop-wrap anchor: {clocks:?}"
    );
    // RTP → decoded left sample.
    let decoded = decode(&listener.packets, 10);
    let first_rtp = decoded[0].0;
    let mut audio = Vec::new();
    for (ts, l, _) in &decoded {
        let at = ts.wrapping_sub(first_rtp) as usize;
        audio.resize(at, 0.0);
        audio.extend_from_slice(l);
    }
    let sample = |rtp: u32| audio.get(rtp.wrapping_sub(first_rtp) as usize).copied();

    // Every beat an anchor predicts (before the next anchor applies) must be a click onset.
    let mut checked = 0;
    for w in clocks.windows(2) {
        let (a, b): (&StreamClock, &StreamClock) = (&w[0], &w[1]);
        if !a.playing {
            continue;
        }
        let span = b.rtp.wrapping_sub(a.rtp) as f64 / 48_000.0 * 2.0; // beats until b
        let mut beat = a.position.0.ceil();
        while beat < a.position.0 + span {
            let r = a
                .rtp
                .wrapping_add(((beat - a.position.0) * 24_000.0).round() as u32);
            let window: Option<Vec<f32>> = (0..2400u32)
                .map(|k| sample(r.wrapping_sub(1200).wrapping_add(k)))
                .collect();
            if let Some(window) = window {
                let peak = window.iter().fold(0f32, |m, x| m.max(x.abs()));
                assert!(peak > 0.05, "no click around beat {beat} (rtp {r})");
                let onset = window.iter().position(|x| x.abs() > 0.2 * peak).unwrap();
                let err = onset as i64 - 1200;
                assert!(
                    err.abs() <= 96,
                    "beat {beat}: click heard {err} samples from its anchored rtp"
                );
                checked += 1;
            }
            beat += 1.0;
        }
    }
    assert!(checked >= 3, "only {checked} clicks checked");
}
