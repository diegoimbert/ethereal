//! Offline render tests for the built-in devices. Every `process` call runs under
//! `assert_no_alloc` (debug builds abort on allocation on the audio path).

use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, MediaId, ParamId};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, ProcessStatus, TransportInfo,
};
use ether_devices::{
    Compressor, Delay, NoSamples, SampleResolver, Sampler, Synth, compressor, delay, sampler, synth,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn prepared<D: Device + ?Sized>(d: &mut D) {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
}

/// Render `frames` of stereo output from `input` (per channel, may be empty = silence).
/// `events` are `(absolute frame, kind)`, sorted.
fn render<D: Device + ?Sized>(
    d: &mut D,
    input: &[Vec<f32>; 2],
    events: &[(usize, EventKind)],
    frames: usize,
    bpm: f64,
) -> [Vec<f32>; 2] {
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let zeros = vec![0.0f32; BLOCK];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        bpm,
        beats_per_sample: bpm / 60.0 / SR as f64,
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
        let in_l = input[0].get(pos..pos + n).unwrap_or(&zeros[..n]);
        let in_r = input[1].get(pos..pos + n).unwrap_or(&zeros[..n]);
        let inputs: [&[f32]; 2] = [in_l, in_r];
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

fn silence(frames: usize) -> [Vec<f32>; 2] {
    [vec![0.0; frames], vec![0.0; frames]]
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
        .collect()
}

fn note_on(id: u32, key: u8) -> EventKind {
    EventKind::NoteOn {
        note_id: id,
        channel: 0,
        key,
        velocity: 1.0,
    }
}

fn note_off(id: u32, key: u8) -> EventKind {
    EventKind::NoteOff {
        note_id: id,
        channel: 0,
        key,
        velocity: 0.0,
    }
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param { param: id, value }
}

/// Estimated frequency from rising zero crossings.
fn zero_crossing_hz(x: &[f32]) -> f32 {
    let crossings: Vec<usize> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .collect();
    let span = (crossings.last().unwrap() - crossings[0]) as f32;
    (crossings.len() - 1) as f32 * SR / span
}

// ---------------------------------------------------------------------------------------
// Descriptors

const TYPES: [BuiltinDeviceType; 9] = BuiltinDeviceType::ALL;

fn builtin(t: BuiltinDeviceType) -> BuiltinDevice {
    BuiltinDevice::new(t)
}

#[test]
fn descriptors_are_consistent_and_identical_per_type() {
    assert_eq!(
        ether_devices::all_descriptors().len(),
        BuiltinDeviceType::ALL.len()
    );
    for t in TYPES {
        let desc = ether_devices::descriptor(t);
        // Every instance (fresh, or after param changes / prepare) reports the same list.
        let mut a = ether_devices::create(&builtin(t), &NoSamples);
        let b = ether_devices::create(&builtin(t), &NoSamples);
        for p in &desc.params {
            a.set_param(p.id, p.max);
        }
        prepared(&mut *a);
        assert_eq!(a.descriptor(), desc, "{t:?}");
        assert_eq!(b.descriptor(), desc, "{t:?}");

        for (i, p) in desc.params.iter().enumerate() {
            assert_eq!(p.id, ParamId(i as u32), "{t:?}: ids are dense and ordered");
            assert!(
                p.min < p.max && (p.min..=p.max).contains(&p.default),
                "{t:?} {p:?}"
            );
            assert_eq!(b.param(p.id), Some(p.default), "{t:?} {}", p.name);
            if let Some(labels) = &p.labels {
                assert_eq!(p.max - p.min, (labels.len() - 1) as f64);
            }
            // Normalized mapping round-trips at the default.
            let n = p.to_normalized(p.default);
            assert!((p.to_plain(n) - p.default).abs() < 1e-6 * p.max.abs().max(1.0));
        }
        assert_eq!(b.param(ParamId(999)), None);
    }
    let synth = ether_devices::descriptor(BuiltinDeviceType::Synth);
    assert_eq!(synth.category, DeviceCategory::Instrument);
    assert!(synth.midi_input && synth.audio_inputs == 0);
    let comp = ether_devices::descriptor(BuiltinDeviceType::Compressor);
    assert_eq!(comp.category, DeviceCategory::AudioEffect);
}

#[test]
fn set_param_clamps() {
    let mut s = Synth::new();
    s.set_param(synth::params::CUTOFF, 1e9);
    assert_eq!(s.param(synth::params::CUTOFF), Some(20000.0));
    s.set_param(synth::params::CUTOFF, f64::NAN);
    assert_eq!(s.param(synth::params::CUTOFF), Some(20.0));
    s.set_param(ParamId(999), 1.0); // ignored
}

// ---------------------------------------------------------------------------------------
// Synth

fn synth_with(waveform: f64) -> Synth {
    let mut s = Synth::new();
    prepared(&mut s);
    s.set_param(synth::params::WAVEFORM, waveform);
    s.set_param(synth::params::CUTOFF, 20000.0);
    s
}

#[test]
fn synth_silent_without_notes() {
    let mut s = synth_with(1.0);
    let out = render(&mut s, &silence(0), &[], 2048, 120.0);
    assert_eq!(peak(&out[0]), 0.0);
}

#[test]
fn synth_plays_pitch_for_every_waveform() {
    for w in 0..4 {
        let mut s = synth_with(w as f64);
        let out = render(
            &mut s,
            &silence(0),
            &[(0, note_on(1, 69))],
            SR as usize / 2,
            120.0,
        );
        let tail = &out[0][4800..];
        assert!(out[0].iter().all(|v| v.is_finite()));
        assert!(rms(tail) > 0.01, "waveform {w} audible");
        assert!(peak(&out[0]) < 1.0, "waveform {w} in range");
        assert_eq!(out[0], out[1], "mono voice on both channels");
        let hz = zero_crossing_hz(tail);
        assert!((hz - 440.0).abs() < 2.0, "waveform {w}: {hz} Hz");
    }
}

#[test]
fn synth_transpose_shifts_pitch() {
    let mut s = synth_with(0.0);
    s.set_param(synth::params::TRANSPOSE, 12.0);
    let out = render(&mut s, &silence(0), &[(0, note_on(1, 57))], 24_000, 120.0);
    let hz = zero_crossing_hz(&out[0][4800..]);
    assert!((hz - 440.0).abs() < 2.0, "{hz}");
}

#[test]
fn synth_note_off_releases_to_silence() {
    let mut s = synth_with(1.0);
    s.set_param(synth::params::RELEASE, 50.0);
    let events = [(0, note_on(7, 60)), (4800, note_off(7, 60))];
    let out = render(&mut s, &silence(0), &events, 24_000, 120.0);
    assert!(rms(&out[0][2400..4800]) > 0.01);
    // 50 ms release reaches -80 dB well before 150 ms after note-off.
    assert!(peak(&out[0][12_000..]) < 1e-4);
}

#[test]
fn synth_note_off_by_key_when_ids_differ() {
    let mut s = synth_with(1.0);
    s.set_param(synth::params::RELEASE, 10.0);
    let events = [(0, note_on(1, 60)), (1000, note_off(u32::MAX, 60))];
    let out = render(&mut s, &silence(0), &events, 12_000, 120.0);
    assert!(peak(&out[0][8000..]) < 1e-4);
}

#[test]
fn synth_is_polyphonic() {
    let mut one = synth_with(0.0);
    let single = render(&mut one, &silence(0), &[(0, note_on(1, 60))], 9600, 120.0);
    let mut s = synth_with(0.0);
    let events = [
        (0, note_on(1, 60)),
        (0, note_on(2, 64)),
        (0, note_on(3, 67)),
    ];
    let chord = render(&mut s, &silence(0), &events, 9600, 120.0);
    let r1 = rms(&single[0][4800..]);
    let r3 = rms(&chord[0][4800..]);
    // Three uncorrelated sines: ~sqrt(3) x the RMS of one.
    assert!(r3 > r1 * 1.5, "{r1} vs {r3}");
    // Stealing: more notes than voices never panics and stays bounded.
    let many: Vec<(usize, EventKind)> = (0..40)
        .map(|i| (i * 10, note_on(i as u32, 40 + i as u8)))
        .collect();
    let mut s = synth_with(1.0);
    let out = render(&mut s, &silence(0), &many, 4800, 120.0);
    assert!(out[0].iter().all(|v| v.is_finite() && v.abs() < 4.0));
}

#[test]
fn synth_filter_removes_highs() {
    let hf = |cutoff: f64| {
        let mut s = synth_with(1.0);
        s.set_param(synth::params::CUTOFF, cutoff);
        let out = render(&mut s, &silence(0), &[(0, note_on(1, 48))], 24_000, 120.0);
        let diff: Vec<f32> = out[0].windows(2).map(|w| w[1] - w[0]).collect();
        rms(&diff[4800..])
    };
    let open = hf(20000.0);
    let closed = hf(300.0);
    assert!(closed < open * 0.2, "{open} vs {closed}");
}

#[test]
fn synth_param_events_are_applied() {
    let mut s = synth_with(0.0);
    let events = [
        (0, note_on(1, 69)),
        (100, param(synth::params::VOLUME, -60.0)),
    ];
    let out = render(&mut s, &silence(0), &events, 24_000, 120.0);
    assert_eq!(s.param(synth::params::VOLUME), Some(-60.0));
    assert!(peak(&out[0][12_000..]) < 0.002);
}

#[test]
fn synth_reset_and_status() {
    let mut s = synth_with(0.0);
    render(&mut s, &silence(0), &[(0, note_on(1, 60))], 512, 120.0);
    s.reset();
    let out = render(&mut s, &silence(0), &[], 512, 120.0);
    assert_eq!(peak(&out[0]), 0.0);
}

// ---------------------------------------------------------------------------------------
// Sampler

/// Ramp sample: frame i = i / len (mono) or (i/len, -i/len) (stereo).
struct Ramp {
    frames: usize,
    channels: u16,
}

impl AudioSource for Ramp {
    fn channels(&self) -> u16 {
        self.channels
    }
    fn frames(&self) -> u64 {
        self.frames as u64
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let sign = if channel == 1 { -1.0 } else { 1.0 };
        for (k, o) in out.iter_mut().enumerate() {
            let i = start as usize + k;
            *o = if i < self.frames {
                sign * i as f32 / self.frames as f32
            } else {
                0.0
            };
        }
        true
    }
}

struct OneSample(Arc<dyn AudioSource>);

impl SampleResolver for OneSample {
    fn resolve(&self, _media: MediaId) -> Option<Arc<dyn AudioSource>> {
        Some(self.0.clone())
    }
}

fn ramp_sampler(frames: usize, channels: u16, mode: f64) -> Sampler {
    let mut s = Sampler::new(Some(Arc::new(Ramp { frames, channels })));
    prepared(&mut s);
    s.set_param(sampler::params::MODE, mode);
    s.set_param(sampler::params::ATTACK, 0.0);
    s
}

fn last_nonzero(x: &[f32]) -> usize {
    x.iter().rposition(|v| *v != 0.0).unwrap_or(0)
}

#[test]
fn sampler_one_shot_plays_whole_sample_at_original_pitch() {
    let len = 3000;
    let mut s = ramp_sampler(len, 2, 0.0);
    // Note-off right away is ignored in one-shot mode; key doesn't change pitch.
    let events = [(100, note_on(1, 30)), (110, note_off(1, 30))];
    let out = render(&mut s, &silence(0), &events, 8000, 120.0);
    for k in [0, 1, 500, 2999] {
        let expect = k as f32 / len as f32;
        assert!((out[0][100 + k] - expect).abs() < 1e-5, "frame {k}");
        assert!((out[1][100 + k] + expect).abs() < 1e-5, "stereo frame {k}");
    }
    assert_eq!(last_nonzero(&out[0]), 100 + len - 1);
}

#[test]
fn sampler_pitched_follows_root_key() {
    let len = 4000;
    let mut s = ramp_sampler(len, 1, 1.0);
    s.set_param(sampler::params::ROOT_KEY, 60.0);
    let out = render(&mut s, &silence(0), &[(0, note_on(1, 72))], 8000, 120.0);
    // An octave up plays twice as fast: half the length, mono to both channels.
    let end = last_nonzero(&out[0]);
    assert!((end as i64 - 2000).abs() <= 1, "{end}");
    assert!((out[0][1000] - 0.5).abs() < 1e-4);
    assert_eq!(out[0], out[1]);

    // An octave down: linear interpolation between frames.
    let mut s = ramp_sampler(len, 1, 1.0);
    let out = render(&mut s, &silence(0), &[(0, note_on(1, 48))], 9000, 120.0);
    assert!((out[0][1001] - 500.5 / len as f32).abs() < 1e-5);
    assert!((last_nonzero(&out[0]) as i64 - 8000).abs() <= 2);
}

#[test]
fn sampler_pitched_note_off_releases() {
    let mut s = ramp_sampler(48_000, 1, 1.0);
    s.set_param(sampler::params::RELEASE, 10.0);
    let events = [(0, note_on(1, 60)), (1000, note_off(1, 60))];
    let out = render(&mut s, &silence(0), &events, 4000, 120.0);
    assert!(out[0][999] > 0.0);
    assert_eq!(peak(&out[0][2000..]), 0.0);
}

#[test]
fn sampler_without_sample_is_silent() {
    let mut s = ether_devices::create(&BuiltinDevice::Sampler { sample: None, slices: Default::default() }, &NoSamples);
    prepared(&mut *s);
    let out = render(&mut *s, &silence(0), &[(0, note_on(1, 60))], 1024, 120.0);
    assert_eq!(peak(&out[0]), 0.0);

    let media: MediaId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let resolver = OneSample(Arc::new(Ramp {
        frames: 1000,
        channels: 1,
    }));
    let mut s = ether_devices::create(
        &BuiltinDevice::Sampler { sample: Some(media), slices: Default::default() },
        &resolver,
    );
    prepared(&mut *s);
    let out = render(&mut *s, &silence(0), &[(0, note_on(1, 60))], 1024, 120.0);
    assert!(peak(&out[0]) > 0.5);
}

#[test]
fn sampler_polyphony_sums_voices() {
    let mut s = ramp_sampler(4000, 1, 0.0);
    let events = [(0, note_on(1, 60)), (0, note_on(2, 62))];
    let out = render(&mut s, &silence(0), &events, 4000, 120.0);
    assert!((out[0][2000] - 1.0).abs() < 1e-4);
    // Choke silences immediately.
    let mut s = ramp_sampler(4000, 1, 0.0);
    let events = [
        (0, note_on(1, 60)),
        (
            500,
            EventKind::NoteChoke {
                note_id: 1,
                channel: 0,
                key: 60,
            },
        ),
    ];
    let out = render(&mut s, &silence(0), &events, 4000, 120.0);
    assert_eq!(peak(&out[0][500..]), 0.0);
}

// ---------------------------------------------------------------------------------------
// Compressor

fn compress(level: f32, setup: impl Fn(&mut Compressor)) -> f32 {
    let mut c = Compressor::new();
    prepared(&mut c);
    setup(&mut c);
    let frames = 48_000;
    let x = sine(1000.0, level, frames);
    let out = render(&mut c, &[x.clone(), x], &[], frames, 120.0);
    peak(&out[0][24_000..])
}

#[test]
fn compressor_leaves_quiet_signals_alone() {
    // -40 dBFS, threshold -18: untouched.
    let a = 0.01;
    let p = compress(a, |_| {});
    assert!((p - a).abs() < a * 0.01, "{p}");
}

#[test]
fn compressor_reduces_loud_signals_by_ratio() {
    // 0 dBFS peak, threshold -20, 4:1 -> 15 dB reduction -> -15 dBFS peak (peak detector,
    // ripple from release on a 1 kHz sine allowed).
    let p = compress(1.0, |c| {
        c.set_param(compressor::params::THRESHOLD, -20.0);
        c.set_param(compressor::params::RATIO, 4.0);
        c.set_param(compressor::params::RELEASE, 500.0);
    });
    let db = 20.0 * p.log10();
    assert!((db + 15.0).abs() < 1.0, "{db} dB");
    // Makeup adds on top.
    let p = compress(1.0, |c| {
        c.set_param(compressor::params::THRESHOLD, -20.0);
        c.set_param(compressor::params::RATIO, 4.0);
        c.set_param(compressor::params::RELEASE, 500.0);
        c.set_param(compressor::params::MAKEUP, 6.0);
    });
    let db = 20.0 * p.log10();
    assert!((db + 9.0).abs() < 1.0, "{db} dB");
}

#[test]
fn compressor_gain_reduction_settles_to_exact_zero() {
    let mut c = Compressor::new();
    prepared(&mut c);
    c.set_param(compressor::params::THRESHOLD, -30.0);
    let frames = 20 * SR as usize;
    let mut x = vec![0.0f32; frames];
    x[..4800].fill(0.9);
    render(&mut c, &[x.clone(), x], &[], frames, 120.0);
    // Exactly 0, not a subnormal tail.
    assert_eq!(c.gain_reduction(), 0.0);
}

#[test]
fn compressor_attack_and_release_times() {
    let mut c = Compressor::new();
    prepared(&mut c);
    c.set_param(compressor::params::THRESHOLD, -30.0);
    c.set_param(compressor::params::RATIO, 20.0);
    c.set_param(compressor::params::ATTACK, 1.0);
    c.set_param(compressor::params::RELEASE, 100.0);
    // Loud burst for 0.1 s, then quiet.
    let frames = 48_000;
    let mut x = vec![0.0f32; frames];
    for (i, v) in x.iter_mut().enumerate() {
        *v = if i < 4800 { 0.9 } else { 0.001 };
    }
    let out = render(&mut c, &[x.clone(), x.clone()], &[], frames, 120.0);
    // Attack: after ~10 time constants the burst is fully compressed.
    let g_attack = out[0][480] / x[480];
    assert!(g_attack < 0.1, "{g_attack}");
    // Release: gain recovers towards unity after the burst (0.5 s = 5 time constants).
    let g_soon = out[0][4800 + 480] / x[4800 + 480];
    let g_late = out[0][frames - 1] / x[frames - 1];
    assert!(g_soon < 0.5 && g_late > 0.95, "{g_soon} {g_late}");
}

// ---------------------------------------------------------------------------------------
// Delay

fn impulse(frames: usize) -> [Vec<f32>; 2] {
    let mut x = vec![0.0; frames];
    x[0] = 1.0;
    [x.clone(), x]
}

#[test]
fn delay_echo_at_time_in_ms() {
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::TIME, 100.0);
    d.set_param(delay::params::FEEDBACK, 50.0);
    d.set_param(delay::params::MIX, 50.0);
    let out = render(&mut d, &impulse(24_000), &[], 24_000, 120.0);
    assert!((out[0][0] - 0.5).abs() < 1e-6, "dry");
    assert!((out[0][4800] - 0.5).abs() < 1e-4, "first echo");
    assert!((out[0][9600] - 0.25).abs() < 1e-4, "second echo (feedback)");
    assert!((out[1][4800] - 0.5).abs() < 1e-4, "right channel");
    let between: f32 = out[0][1..4800].iter().map(|v| v.abs()).sum();
    assert!(between < 1e-6);
}

#[test]
fn delay_synced_to_tempo() {
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::SYNC, 1.0);
    d.set_param(delay::params::DIVISION, 7.0); // 1/4
    d.set_param(delay::params::FEEDBACK, 0.0);
    d.set_param(delay::params::MIX, 100.0);
    // 1/4 at 120 bpm = 0.5 s; at 100 bpm = 0.6 s.
    let out = render(&mut d, &impulse(48_000), &[], 48_000, 120.0);
    assert!((out[0][24_000] - 1.0).abs() < 1e-4);
    assert_eq!(out[0][0], 0.0, "fully wet");
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::SYNC, 1.0);
    d.set_param(delay::params::DIVISION, 7.0);
    d.set_param(delay::params::MIX, 100.0);
    let out = render(&mut d, &impulse(48_000), &[], 48_000, 100.0);
    assert!((out[0][28_800] - 1.0).abs() < 1e-4);
}

#[test]
fn delay_mix_zero_is_dry_and_time_changes_glide() {
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::MIX, 0.0);
    let x = sine(440.0, 0.5, 4800);
    let out = render(&mut d, &[x.clone(), x.clone()], &[], 4800, 120.0);
    assert!(out[0].iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));

    // Automating time mid-stream stays bounded and continuous.
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::MIX, 100.0);
    d.set_param(delay::params::FEEDBACK, 90.0);
    let x = sine(220.0, 0.5, 48_000);
    let events = [
        (12_000, param(delay::params::TIME, 20.0)),
        (24_000, param(delay::params::TIME, 1500.0)),
    ];
    let out = render(&mut d, &[x.clone(), x], &events, 48_000, 120.0);
    assert!(out[0].iter().all(|v| v.is_finite() && v.abs() < 10.0));

    // A time event glides to the new time: an impulse long after lands at the new delay.
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::TIME, 100.0);
    d.set_param(delay::params::FEEDBACK, 0.0);
    d.set_param(delay::params::MIX, 100.0);
    let mut x = vec![0.0f32; 48_000];
    x[24_000] = 1.0;
    let events = [(0, param(delay::params::TIME, 50.0))];
    let out = render(&mut d, &[x.clone(), x], &events, 48_000, 120.0);
    assert_eq!(d.param(delay::params::TIME), Some(50.0));
    assert!((out[0][24_000 + 2400] - 1.0).abs() < 1e-3);
}

#[test]
fn delay_reset_clears_tail() {
    let mut d = Delay::new();
    prepared(&mut d);
    d.set_param(delay::params::MIX, 100.0);
    render(&mut d, &impulse(1000), &[], 1000, 120.0);
    d.reset();
    let out = render(&mut d, &silence(0), &[], 48_000, 120.0);
    assert_eq!(peak(&out[0]), 0.0);
}

#[test]
fn effects_report_continue_and_instruments_silent() {
    let mut s = synth_with(0.0);
    let transport = TransportInfo::STOPPED;
    let mut out_events = EventBuffer::with_capacity(4);
    let mut l = [0.0f32; 64];
    let mut r = [0.0f32; 64];
    let mut outputs: [&mut [f32]; 2] = [&mut l, &mut r];
    let mut ctx = ProcessContext {
        sample_rate: SR,
        frames: 64,
        transport: &transport,
        events: &[],
        out_events: &mut out_events,
    };
    let mut audio = AudioBuffers {
        inputs: &[],
        outputs: &mut outputs,
    };
    assert_eq!(s.process(&mut ctx, &mut audio), ProcessStatus::Silent);
}
