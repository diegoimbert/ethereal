//! Patches written before the pass render as before. Each case
//! uses only params that existed then (by number) and is compared with a fingerprint
//! recorded on the engine before the pass (`print_golden_fingerprints`, run on `dev`):
//! per-block RMS everywhere (robust to another platform's libm) and the exact sample bits on
//! Linux x86-64, where they were recorded. (The Poly Synth's cases live in
//! `instruments_poly_synth.rs`.)

use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{ParamId, SliceSettings};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::{Sampler, Synth};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// A stereo test sample: a decaying 220 Hz-ish tone plus a ramp, 0.5 s.
struct Tone;

impl AudioSource for Tone {
    fn channels(&self) -> u16 {
        2
    }
    fn frames(&self) -> u64 {
        24_000
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (k, o) in out.iter_mut().enumerate() {
            let i = start as usize + k;
            *o = if i < 24_000 {
                let t = i as f32 / SR;
                let tone = (std::f32::consts::TAU * 220.0 * t).sin() * (-3.0 * t).exp();
                let ramp = i as f32 / 24_000.0;
                if channel == 0 {
                    0.6 * tone + 0.2 * ramp
                } else {
                    0.6 * tone - 0.2 * ramp
                }
            } else {
                0.0
            };
        }
        true
    }
}

fn on(key: u8, note_id: u32, velocity: f32) -> EventKind {
    EventKind::NoteOn {
        note_id,
        channel: 0,
        key,
        velocity,
    }
}

fn off(key: u8, note_id: u32) -> EventKind {
    EventKind::NoteOff {
        note_id,
        channel: 0,
        key,
        velocity: 0.0,
    }
}

fn render(d: &mut dyn Device, events: &[(usize, EventKind)], frames: usize) -> [Vec<f32>; 2] {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut evs = EventBuffer::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        bpm: 120.0,
        beats_per_sample: 2.0 / f64::from(SR),
        ..TransportInfo::STOPPED
    };
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        evs.clear();
        for (at, kind) in events {
            if *at >= pos && *at < pos + n {
                evs.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
        }
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: evs.as_slice(),
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &[],
            outputs: &mut outputs,
        };
        assert_no_alloc(|| d.process(&mut ctx, &mut audio));
        pos += n;
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// RMS of each 1 000-sample block of both channels, and a hash of the sample bits.
fn fingerprint([l, r]: &[Vec<f32>; 2]) -> (Vec<f32>, u64) {
    let blocks = l
        .chunks(1_000)
        .zip(r.chunks(1_000))
        .map(|(a, b)| rms(a) + rms(b))
        .collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in l.iter().chain(r) {
        h ^= u64::from(v.to_bits());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (blocks, h)
}

fn chord() -> Vec<(usize, EventKind)> {
    vec![
        (0, on(48, 1, 0.9)),
        (0, on(55, 2, 0.6)),
        (2_000, on(64, 3, 0.3)),
        (20_000, off(48, 1)),
        (20_000, off(55, 2)),
        (30_000, off(64, 3)),
    ]
}

/// The cases: (name, render).
fn cases() -> Vec<(&'static str, [Vec<f32>; 2])> {
    let p = ParamId;
    let synth = |params: &[(u32, f64)]| {
        let mut s = Synth::new();
        for &(id, v) in params {
            s.set_param(p(id), v);
        }
        render(&mut s, &chord(), 36_000)
    };
    let sampler = |params: &[(u32, f64)], slices: Option<SliceSettings>| {
        let src: Arc<dyn AudioSource> = Arc::new(Tone);
        let mut s = match slices {
            Some(sl) => Sampler::with_slices(Some(src), sl),
            None => Sampler::new(Some(src)),
        };
        for &(id, v) in params {
            s.set_param(p(id), v);
        }
        let events = [
            (0, on(60, 1, 1.0)),
            (100, on(67, 2, 0.5)),
            (5_000, on(36, 3, 0.8)),
            (9_000, on(61, 4, 0.7)),
            (15_000, off(60, 1)),
            (20_000, off(67, 2)),
            (22_000, off(36, 3)),
            (23_000, off(61, 4)),
        ];
        render(&mut s, &events, 36_000)
    };
    let slices = SliceSettings {
        enabled: true,
        base_note: 60,
        markers: vec![
            ether_core::protocol::model::Seconds(0.0),
            ether_core::protocol::model::Seconds(0.1),
            ether_core::protocol::model::Seconds(0.25),
        ],
    };
    vec![
        ("synth default", synth(&[])),
        (
            "synth square, resonant",
            synth(&[
                (0, 2.0),
                (1, -12.0),
                (6, 900.0),
                (7, 70.0),
                (3, 80.0),
                (4, 30.0),
            ]),
        ),
        ("sampler default (pitched)", sampler(&[], None)),
        (
            "sampler one-shot, range",
            sampler(&[(0, 0.0), (6, 10.0), (7, 80.0), (2, 5.0), (5, -6.0)], None),
        ),
        (
            "sampler pitched, attack/release, range",
            sampler(
                &[(3, 40.0), (4, 400.0), (6, 20.0), (7, 60.0), (1, 55.0)],
                None,
            ),
        ),
        ("sampler slices", sampler(&[], Some(slices))),
    ]
}

#[test]
#[ignore = "prints the fingerprints to paste into GOLDEN"]
fn print_golden_fingerprints() {
    for (name, out) in cases() {
        let (blocks, hash) = fingerprint(&out);
        println!("    ({name:?}, &{blocks:?}, {hash:#x}),");
    }
}

/// Recorded on `dev` before this pass. The pitched sampler hashes were re-recorded when
/// sample reads became band-limited, the synth ones with its 4-point polyBLEP.
#[rustfmt::skip]
const GOLDEN: &[(&str, &[f32], u64)] = &[
    ("synth default", &[0.27813712, 0.26294354, 0.28318664, 0.24026938, 0.24484167, 0.26388997, 0.22873497, 0.22018507, 0.25455624, 0.21903804, 0.23048343, 0.24841115, 0.21750805, 0.25699583, 0.22034453, 0.21459094, 0.24937214, 0.22163662, 0.21014625, 0.24129285, 0.1629838, 0.093147896, 0.07030836, 0.06373118, 0.059210774, 0.0597732, 0.06014857, 0.060505707, 0.060528934, 0.060363267, 0.04198698, 0.01942349, 0.0092140315, 0.0043633105, 0.0020354157, 0.00093729864], 0xaf73eb95798a2d1),
    ("synth square, resonant", &[0.32148588, 0.2328305, 0.18011588, 0.15900916, 0.20446064, 0.1696999, 0.16016015, 0.19836147, 0.17516923, 0.1559334, 0.19347206, 0.18320936, 0.14777927, 0.19040374, 0.18602434, 0.16279016, 0.18010476, 0.19130254, 0.1629076, 0.1765446, 0.14010012, 0.06838542, 0.054009717, 0.051395576, 0.050595738, 0.049895838, 0.04986695, 0.049738895, 0.049786586, 0.04998174, 0.0348546, 0.016666291, 0.007700285, 0.0035678218, 0.0016498537, 0.00076279696], 0x4bad65968eb490ad),
    ("sampler default (pitched)", &[0.8960354, 0.849738, 0.85007244, 0.74163586, 0.69130844, 0.98138463, 0.9002707, 0.91826427, 0.8337034, 1.2335702, 1.0751002, 0.79206705, 0.9470308, 1.0847653, 0.9548954, 0.76382124, 0.71660686, 0.66557896, 0.64127326, 0.6514386, 0.64005214, 0.60743695, 0.40097666, 0.1485463, 0.021673009, 0.0033435766, 0.00047924282, 6.871275e-5, 9.968204e-6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0x968ee281e22177b3),
    ("sampler one-shot, range", &[0.25501576, 0.23344165, 0.21890485, 0.20827942, 0.20393303, 0.28123122, 0.28032857, 0.28474766, 0.29123035, 0.31273705, 0.33687368, 0.36046034, 0.29736847, 0.14734691, 0.16264305, 0.17818189, 0.19402666, 0.18030839, 0.14576477, 0.14426869, 0.14300689, 0.10300323, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0x972968eeb75a60ef),
    ("sampler pitched, attack/release, range", &[0.2010396, 0.5110223, 0.6016756, 0.5669172, 0.526689, 0.47983596, 0.5868512, 0.5144298, 0.47306955, 0.477368, 0.557958, 0.59785914, 0.5822807, 0.5695118, 0.5592139, 0.52761984, 0.41295305, 0.4047982, 0.39692965, 0.3895499, 0.3828339, 0.37690318, 0.2988332, 0.1841104, 0.113450624, 0.069849715, 0.042935066, 0.026336156, 0.016118884, 0.009845827, 0.006005095, 0.0036597499, 0.0022306344, 0.0012171759, 0.0, 0.0], 0x360ad3b1c941338b),
    ("sampler slices", &[0.8091241, 0.7738042, 0.73294663, 0.68944526, 0.5500833, 0.0, 0.0, 0.0, 0.0, 0.42396337, 0.40772134, 0.38891184, 0.36917114, 0.34926736, 0.33201677, 0.31938332, 0.1182245, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0x821d67b710c4b6c9),
];

#[test]
fn patches_from_before_render_as_before() {
    let cases = cases();
    assert_eq!(GOLDEN.len(), cases.len());
    for ((name, out), (gname, blocks, hash)) in cases.into_iter().zip(GOLDEN) {
        assert_eq!(name, *gname);
        let (now, now_hash) = fingerprint(&out);
        assert_eq!(now.len(), blocks.len());
        for (i, (a, b)) in now.iter().zip(blocks.iter()).enumerate() {
            assert!(
                (a - b).abs() <= 1e-5 + 1e-4 * b.abs(),
                "{name}: block {i}: {a} vs {b}"
            );
        }
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(now_hash, *hash, "{name}: not bit-exact");
        }
    }
}
