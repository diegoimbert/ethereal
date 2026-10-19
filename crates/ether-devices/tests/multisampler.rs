//! Multisampler node (v0.2, `multisampler`): zone selection, velocity crossfades, round
//! robin, pitch, loops with crossfade, live zone swaps. Every `process` and `set_data` call
//! runs under `assert_no_alloc`.

use std::collections::HashMap;
use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{
    BuiltinDevice, Decibels, MediaId, Pan, SampleZone, Seconds, Ulid, Zone,
};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, ProcessStatus, TransportInfo,
};
use ether_devices::SampleResolver;
use ether_devices::multisampler::{self, MultiSampler, ZoneSet, multi_sampler as p};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// A mono source computed from the frame index.
struct FnSource {
    frames: usize,
    f: fn(usize) -> f32,
}

impl AudioSource for FnSource {
    fn channels(&self) -> u16 {
        1
    }
    fn frames(&self) -> u64 {
        self.frames as u64
    }
    fn read(&self, _channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (k, o) in out.iter_mut().enumerate() {
            let i = start as usize + k;
            *o = if i < self.frames { (self.f)(i) } else { 0.0 };
        }
        true
    }
}

fn src(frames: usize, f: fn(usize) -> f32) -> Arc<dyn AudioSource> {
    Arc::new(FnSource { frames, f })
}

fn media(n: u128) -> MediaId {
    MediaId(Ulid(n))
}

struct Sources(HashMap<MediaId, Arc<dyn AudioSource>>);

impl SampleResolver for Sources {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

fn sources(list: &[(u128, Arc<dyn AudioSource>)]) -> Sources {
    Sources(list.iter().map(|(n, s)| (media(*n), s.clone())).collect())
}

fn zone(m: u128, keys: (u8, u8), vels: (u8, u8)) -> SampleZone {
    SampleZone {
        media: Some(media(m)),
        keys: Zone {
            lo: keys.0,
            hi: keys.1,
        },
        velocities: Zone {
            lo: vels.0,
            hi: vels.1,
        },
        ..SampleZone::default()
    }
}

/// A prepared multisampler with instant attack, 0 dB, velocity-insensitive.
fn device(zones: Vec<SampleZone>, srcs: &Sources) -> MultiSampler {
    let mut d = MultiSampler::new(multisampler::zone_set(
        &BuiltinDevice::MultiSampler { zones },
        srcs,
    ));
    d.set_param(p::AMP_ATTACK, 0.0);
    d.set_param(p::VOLUME, 0.0);
    d.set_param(p::VELOCITY, 0.0);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

fn on(key: u8, velocity: f32) -> EventKind {
    EventKind::NoteOn {
        note_id: key as u32,
        channel: 0,
        key,
        velocity,
    }
}

fn off(key: u8) -> EventKind {
    EventKind::NoteOff {
        note_id: key as u32,
        channel: 0,
        key,
        velocity: 0.0,
    }
}

/// Render `frames` stereo frames; `events` are `(absolute frame, kind)`, sorted.
fn render(d: &mut dyn Device, frames: usize, events: &[(usize, EventKind)]) -> [Vec<f32>; 2] {
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        block_events.clear();
        for (at, kind) in events {
            if *at >= pos && *at < pos + n {
                block_events.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
        }
        let inputs: [&[f32]; 0] = [];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &block_events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut audio);
        });
        pos += n;
    }
    out
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn zones_select_by_key_and_velocity() {
    let s = sources(&[
        (1, src(48_000, |_| 0.1)),
        (2, src(48_000, |_| 0.2)),
        (3, src(48_000, |_| 0.3)),
    ]);
    let mut d = device(
        vec![
            zone(1, (0, 59), (1, 127)),
            zone(2, (60, 127), (1, 63)),
            zone(3, (60, 127), (64, 127)),
        ],
        &s,
    );
    let out = render(
        &mut d,
        4000,
        &[
            (0, on(40, 1.0)),
            (
                500,
                EventKind::NoteChoke {
                    note_id: 40,
                    channel: 0,
                    key: 40,
                },
            ),
            (1000, on(70, 0.3)),
            (
                1500,
                EventKind::NoteChoke {
                    note_id: 70,
                    channel: 0,
                    key: 70,
                },
            ),
            (2000, on(70, 0.9)),
        ],
    );
    // Pitched up (key 40 vs root 60 is down; DC is DC): only the zone's level matters.
    assert!(close(out[0][100], 0.1), "{}", out[0][100]);
    assert!(close(out[1][100], 0.1));
    assert!(close(out[0][1100], 0.2), "{}", out[0][1100]);
    assert!(close(out[0][2100], 0.3), "{}", out[0][2100]);
    // A key outside every zone stays silent.
    let s2 = sources(&[(1, src(48_000, |_| 0.1))]);
    let mut d = device(vec![zone(1, (0, 59), (1, 127))], &s2);
    let out = render(&mut d, 1000, &[(0, on(80, 1.0))]);
    assert!(out[0].iter().all(|x| *x == 0.0));
}

#[test]
fn round_robin_plays_zones_in_order() {
    let s = sources(&[
        (1, src(48_000, |_| 0.1)),
        (2, src(48_000, |_| 0.2)),
        (3, src(48_000, |_| 0.3)),
    ]);
    let rr = |m| SampleZone {
        round_robin: 1,
        ..zone(m, (0, 127), (1, 127))
    };
    let mut d = device(vec![rr(1), rr(2), rr(3)], &s);
    let choke = |k: u8| EventKind::NoteChoke {
        note_id: k as u32,
        channel: 0,
        key: k,
    };
    let mut events = Vec::new();
    for i in 0..4 {
        events.push((i * 1000, on(60, 1.0)));
        events.push((i * 1000 + 500, choke(60)));
    }
    let out = render(&mut d, 4000, &events);
    let levels: Vec<f32> = (0..4).map(|i| out[0][i * 1000 + 100]).collect();
    for (got, want) in levels.iter().zip([0.1, 0.2, 0.3, 0.1]) {
        assert!(close(*got, want), "{levels:?}");
    }
    // Random never repeats the previous zone.
    d.set_param(p::ROUND_ROBIN, 1.0);
    let mut events = Vec::new();
    for i in 0..20 {
        events.push((i * 1000, on(60, 1.0)));
        events.push((i * 1000 + 500, choke(60)));
    }
    let out = render(&mut d, 20_000, &events);
    let levels: Vec<f32> = (0..20).map(|i| out[0][i * 1000 + 100]).collect();
    for w in levels.windows(2) {
        assert!(!close(w[0], w[1]), "{levels:?}");
    }
}

#[test]
fn velocity_crossfade_between_layers() {
    let s = sources(&[(1, src(48_000, |_| 0.5)), (2, src(48_000, |_| 0.25))]);
    // Soft 1..=80 and loud 60..=127 overlap on 60..=80.
    let level = |vel: u8| {
        let mut d = device(
            vec![zone(1, (0, 127), (1, 80)), zone(2, (0, 127), (60, 127))],
            &s,
        );
        render(&mut d, 600, &[(0, on(60, vel as f32 / 127.0))])[0][300]
    };
    assert!(close(level(30), 0.5));
    assert!(close(level(110), 0.25));
    // Equal-power across the overlap: the soft layer fades out as the loud one fades in.
    for v in 60..=80u8 {
        let t = (v as f32 - 60.0 + 0.5) / (80.0 - 60.0 + 1.0);
        let want = 0.5 * (t * std::f32::consts::FRAC_PI_2).cos()
            + 0.25 * (t * std::f32::consts::FRAC_PI_2).sin();
        assert!(close(level(v), want), "{v}: {} vs {want}", level(v));
    }
    assert!(close(level(81), 0.25));
}

#[test]
fn zone_pitch_follows_root_key_and_tune() {
    // Ramp source: the output tells the read position.
    let s = sources(&[(1, src(48_000, |i| i as f32 / 48_000.0))]);
    let mut d = device(vec![zone(1, (0, 127), (1, 127))], &s);
    let out = render(&mut d, 1000, &[(0, on(72, 1.0))]);
    // An octave up reads two frames per output frame.
    let step = out[0][501] - out[0][500];
    assert!((step * 48_000.0 - 2.0).abs() < 1e-2, "{}", step * 48_000.0);
    let tuned = SampleZone {
        tune_cents: 100.0,
        ..zone(1, (0, 127), (1, 127))
    };
    let mut d = device(vec![tuned], &s);
    let out = render(&mut d, 1000, &[(0, on(60, 1.0))]);
    let step = (out[0][501] - out[0][500]) * 48_000.0;
    assert!((step - 2f32.powf(1.0 / 12.0)).abs() < 1e-2, "{step}");
}

fn sine(i: usize) -> f32 {
    (i as f32 * std::f32::consts::TAU / 97.3).sin() * 0.5
}

fn looped(xfade: f64) -> [Vec<f32>; 2] {
    let s = sources(&[(1, src(48_000, sine))]);
    let z = SampleZone {
        looping: true,
        loop_start: Seconds(4800.0 / 48_000.0),
        loop_end: Seconds(8800.0 / 48_000.0),
        loop_crossfade: Seconds(xfade),
        ..zone(1, (0, 127), (1, 127))
    };
    let mut d = device(vec![z], &s);
    render(&mut d, 30_000, &[(0, on(60, 1.0))])
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn loop_crossfade_is_continuous() {
    // The sine's natural largest step is 2π·0.5/97.3 ≈ 0.032.
    let natural = std::f32::consts::TAU * 0.5 / 97.3;
    let hard = looped(0.0);
    // Without a crossfade the wrap jumps (4000 frames = 41.1 periods: phase mismatch).
    assert!(
        max_step(&hard[0][100..]) > 3.0 * natural,
        "{}",
        max_step(&hard[0][100..])
    );
    // Still sounding well past the loop end (looped several times).
    assert!(hard[0][25_000..].iter().any(|x| x.abs() > 0.1));
    let smooth = looped(0.01);
    let m = max_step(&smooth[0][100..]);
    assert!(m < 1.3 * natural, "max step {m} vs natural {natural}");
    assert!(smooth[0][25_000..].iter().any(|x| x.abs() > 0.1));
    assert!(smooth[0].iter().all(|x| x.is_finite()));
}

#[test]
fn release_leaves_the_loop_and_ends() {
    let s = sources(&[(1, src(20_000, |_| 0.5))]);
    let z = SampleZone {
        looping: true,
        loop_start: Seconds(0.05),
        loop_end: Seconds(0.1),
        end: Some(Seconds(0.3)),
        ..zone(1, (0, 127), (1, 127))
    };
    let mut d = device(vec![z], &s);
    d.set_param(p::AMP_RELEASE, 20_000.0);
    let out = render(&mut d, 48_000, &[(0, on(60, 1.0)), (24_000, off(60))]);
    // Held for 0.5 s (loop), then plays to the zone end (0.3 s of source) and stops.
    assert!(out[0][23_000] > 0.4);
    assert!(out[0][24_000 + 9_000] > 0.0);
    assert!(out[0][24_000 + 14_500..].iter().all(|x| *x == 0.0));
}

#[test]
fn live_zone_swap_keeps_or_fades_voices_without_allocating() {
    let s = sources(&[(1, src(96_000, |_| 0.5)), (2, src(96_000, |_| 0.25))]);
    let mut d = device(vec![zone(1, (0, 127), (1, 127))], &s);
    let a = render(&mut d, 1000, &[(0, on(60, 1.0))]);
    assert!(close(a[0][999], 0.5));
    // New set, same media (e.g. a key range edit): the voice goes on.
    let same = ZoneSet::new(
        &[zone(1, (0, 72), (1, 127)), zone(2, (73, 127), (1, 127))],
        |m| s.resolve(m),
    );
    let data: ether_core::node::NodeData = Box::new(same);
    let old = assert_no_alloc(|| d.set_data(data));
    drop(old);
    let b = render(&mut d, 1000, &[]);
    assert!(close(b[0][0], 0.5) && close(b[0][999], 0.5));
    // New set without that media: the voice fades out within a few milliseconds.
    let other = ZoneSet::new(&[zone(2, (0, 127), (1, 127))], |m| s.resolve(m));
    let data: ether_core::node::NodeData = Box::new(other);
    let old = assert_no_alloc(|| d.set_data(data));
    drop(old);
    let c = render(&mut d, 1000, &[]);
    assert!(c[0][0] > 0.4);
    assert!(c[0][200..].iter().all(|x| *x == 0.0));
    // Notes after the swap use the new zones.
    let e = render(&mut d, 1000, &[(0, on(60, 1.0))]);
    assert!(close(e[0][500], 0.25));
    // Wrong data is handed back untouched.
    let back = d.set_data(Box::new(5u32)).expect("rejected");
    assert_eq!(*back.downcast::<u32>().unwrap(), 5);
}

#[test]
fn missing_media_and_empty_zones_are_silent() {
    let s = sources(&[]);
    let empty = SampleZone::default();
    let mut d = device(vec![zone(9, (0, 127), (1, 127)), empty], &s);
    let out = render(&mut d, 1000, &[(0, on(60, 1.0))]);
    assert!(out[0].iter().all(|x| *x == 0.0));
}

#[test]
fn extreme_params_stay_finite_and_voices_are_bounded() {
    let s = sources(&[(1, src(48_000, |i| ((i * 7919) % 101) as f32 / 50.0 - 1.0))]);
    let z = SampleZone {
        gain: Decibels(24.0),
        pan: Pan(-1.0),
        tune_cents: 100.0,
        looping: true,
        loop_start: Seconds(0.01),
        loop_end: Seconds(0.02),
        loop_crossfade: Seconds(1.0),
        ..zone(1, (0, 127), (1, 127))
    };
    for filter in 0..5 {
        let mut d = device(vec![z.clone(), z.clone()], &s);
        d.set_param(p::FILTER_TYPE, filter as f64);
        d.set_param(p::CUTOFF, 20.0);
        d.set_param(p::RESONANCE, 100.0);
        d.set_param(p::FILTER_ENV_AMOUNT, 100.0);
        d.set_param(p::KEY_TRACKING, 100.0);
        d.set_param(p::TRANSPOSE, 48.0);
        d.set_param(p::GLIDE, 2000.0);
        d.set_param(p::VOICES, 8.0);
        let events: Vec<(usize, EventKind)> = (0..40)
            .map(|i| (i * 100, on((i * 3 % 128) as u8, 1.0)))
            .collect();
        let out = render(&mut d, 12_000, &events);
        assert!(
            out.iter().flatten().all(|x| x.is_finite()),
            "filter {filter}"
        );
        // Hard-left zone pan.
        assert!(out[1].iter().all(|x| *x == 0.0), "filter {filter}");
        assert!(d.active_voices() <= 8, "{} voices", d.active_voices());
        // Automation sweep (sample-accurate param events).
        let sweep: Vec<(usize, EventKind)> = (0..50)
            .map(|i| {
                (
                    i * 37,
                    EventKind::Param {
                        param: p::CUTOFF,
                        value: if i % 2 == 0 { 20.0 } else { 20_000.0 },
                    },
                )
            })
            .collect();
        let out = render(&mut d, 2_000, &sweep);
        assert!(out.iter().flatten().all(|x| x.is_finite()));
    }
}

#[test]
fn polyphony_limit_steals_voices() {
    let s = sources(&[(1, src(96_000, |_| 0.01))]);
    let mut d = MultiSampler::new(ZoneSet::new(&[zone(1, (0, 127), (1, 127))], |m| {
        s.resolve(m)
    }));
    d.set_param(p::AMP_ATTACK, 0.0);
    d.set_param(p::VOICES, 4.0);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    let events: Vec<(usize, EventKind)> =
        (0..10).map(|i| (i * 10, on(40 + i as u8, 1.0))).collect();
    render(&mut d, 1000, &events);
    assert_eq!(d.active_voices(), 4);
}

#[test]
fn silent_without_notes() {
    let s = sources(&[(1, src(48_000, |_| 0.5))]);
    let mut d = device(vec![zone(1, (0, 127), (1, 127))], &s);
    let transport = TransportInfo::STOPPED;
    let mut out_events = EventBuffer::with_capacity(4);
    let mut l = vec![1.0f32; 64];
    let mut r = vec![1.0f32; 64];
    let mut outputs: [&mut [f32]; 2] = [&mut l, &mut r];
    let inputs: [&[f32]; 0] = [];
    let mut ctx = ProcessContext {
        sample_rate: SR,
        frames: 64,
        transport: &transport,
        events: &[],
        out_events: &mut out_events,
    };
    let mut audio = AudioBuffers {
        inputs: &inputs,
        outputs: &mut outputs,
    };
    let status = assert_no_alloc(|| d.process(&mut ctx, &mut audio));
    assert_eq!(status, ProcessStatus::Silent);
    assert!(l.iter().all(|x| *x == 0.0));
}
