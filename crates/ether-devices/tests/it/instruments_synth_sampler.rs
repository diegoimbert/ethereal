//! What the pass added to the Synth (fine, glide, velocity,
//! filter envelope and key tracking) and the Sampler (ADSR, fine, velocity, voices,
//! reverse, forward / ping-pong loops with crossfade, filter). Every render runs under
//! `assert_no_alloc`. That the defaults sound as before is `instruments_golden.rs`.

use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::ParamId;
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::{Sampler, Synth, sampler, synth};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

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

/// Frequency from positive-going zero crossings (near-sine signals).
fn frequency(x: &[f32]) -> f32 {
    let c: Vec<f32> = (1..x.len())
        .filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(c.len() > 2, "no periodic signal");
    (c.len() - 1) as f32 * SR / (c[c.len() - 1] - c[0])
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Energy above ~4 kHz relative to the total (first difference ≈ high-pass).
fn brightness(x: &[f32]) -> f32 {
    let d: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&d) / rms(x).max(1e-9)
}

// --- synth ------------------------------------------------------------------------------

fn sine_synth(params: &[(ParamId, f64)]) -> Synth {
    let mut s = Synth::new();
    s.set_param(synth::params::WAVEFORM, 0.0);
    s.set_param(synth::params::CUTOFF, 20_000.0);
    s.set_param(synth::params::RESONANCE, 0.0);
    s.set_param(synth::params::SUSTAIN, 100.0);
    for &(id, v) in params {
        s.set_param(id, v);
    }
    s
}

#[test]
fn synth_fine_tunes_in_cents() {
    let mut s = sine_synth(&[(synth::params::FINE, 50.0)]);
    let [l, _] = render(&mut s, &[(0, on(69, 1, 1.0))], 48_000);
    let f = frequency(&l[4_800..]);
    let want = 440.0 * (50.0f32 / 1200.0).exp2();
    assert!((f / want - 1.0).abs() < 1e-3, "{f} vs {want}");
}

#[test]
fn synth_transpose_shifts_the_whole_synth() {
    let hz = |params: &[(ParamId, f64)]| {
        let mut s = sine_synth(params);
        let [l, _] = render(&mut s, &[(0, on(57, 1, 1.0))], 24_000);
        frequency(&l[4_800..])
    };
    let close = |got: f32, want: f32| (got / want - 1.0).abs() < 1e-3;
    let up = hz(&[(synth::params::TRANSPOSE, 12.0)]);
    assert!(close(up, 440.0), "+12 st doubles: {up}");
    let both = hz(&[
        (synth::params::TRANSPOSE, -12.0),
        (synth::params::FINE, -25.0),
    ]);
    assert!(close(both, 110.0 * (-25.0f32 / 1200.0).exp2()), "{both}");
}

/// The `Mode` events of a legato phrase: 57, then 69 overlapping it (at 24 000), 69
/// released at 36 000 while 57 is still held, 57 released at 60 000.
fn phrase(mode: f64, glide: f64) -> Vec<f32> {
    let mut s = sine_synth(&[
        (synth::params::VOICE_MODE, mode),
        (synth::params::GLIDE, glide),
        (synth::params::ATTACK, 200.0),
        (synth::params::DECAY, 50.0),
        (synth::params::SUSTAIN, 20.0),
        (synth::params::RELEASE, 5.0),
    ]);
    let [l, _] = render(
        &mut s,
        &[
            (0, on(57, 1, 1.0)),
            (24_000, on(69, 2, 1.0)),
            (36_000, off(69, 2)),
            (60_000, off(57, 1)),
        ],
        72_000,
    );
    l
}

#[test]
fn synth_legato_keeps_one_voice_without_retriggering_and_returns_to_the_held_note() {
    let l = phrase(2.0, 0.0);
    // No new attack: still at the sustain level 40 ms into the second note.
    let level = rms(&l[26_000..26_400]) / rms(&l[23_000..23_400]);
    assert!((level - 1.0).abs() < 0.2, "legato: {level}");
    // One voice: a clean 440 Hz (two voices would beat 220 + 440).
    let f = frequency(&l[24_400..35_000]);
    assert!((f / 440.0 - 1.0).abs() < 1e-3, "takes over: {f}");
    // Releasing 69 returns to the held 57, still without a new attack.
    let f = frequency(&l[36_400..59_000]);
    assert!((f / 220.0 - 1.0).abs() < 1e-3, "back to the held key: {f}");
    let level = rms(&l[38_000..38_400]) / rms(&l[23_000..23_400]);
    assert!((level - 1.0).abs() < 0.2, "no retrigger on return: {level}");
    // Releasing the last key releases the voice.
    assert!(rms(&l[66_000..]) < 1e-4, "released");
}

#[test]
fn synth_mono_retriggers_and_poly_layers() {
    let mono = phrase(1.0, 0.0);
    let level = rms(&mono[26_000..26_400]) / rms(&mono[23_000..23_400]);
    assert!(level > 1.5, "mono retriggers: {level}");
    let f = frequency(&mono[30_000..35_000]);
    assert!((f / 440.0 - 1.0).abs() < 1e-3, "one voice: {f}");
    // Poly (the default): both notes sound, louder than either alone.
    let poly = phrase(0.0, 0.0);
    let both = rms(&poly[30_000..35_000]);
    let one = rms(&phrase(2.0, 0.0)[30_000..35_000]);
    assert!(both > one * 1.3, "poly layers: {both} vs {one}");
}

#[test]
fn synth_legato_glides_only_between_overlapping_notes() {
    let l = phrase(2.0, 100.0);
    let mid = frequency(&l[24_600..26_000]);
    assert!(mid > 240.0 && mid < 420.0, "gliding up: {mid}");
    let end = frequency(&l[33_000..35_900]);
    assert!((end / 440.0 - 1.0).abs() < 3e-3, "arrived: {end}");
    let back = frequency(&l[36_600..38_000]);
    assert!(back > 230.0 && back < 420.0, "gliding back: {back}");
    // A detached note after silence starts at its own pitch (no glide from the last one).
    let mut s = sine_synth(&[
        (synth::params::VOICE_MODE, 2.0),
        (synth::params::GLIDE, 300.0),
        (synth::params::RELEASE, 5.0),
    ]);
    let [l, _] = render(
        &mut s,
        &[
            (0, on(57, 1, 1.0)),
            (9_600, off(57, 1)),
            (24_000, on(69, 2, 1.0)),
        ],
        36_000,
    );
    let f = frequency(&l[24_200..25_600]);
    assert!((f / 440.0 - 1.0).abs() < 0.01, "detached: no glide {f}");
}

#[test]
fn synth_glides_from_the_last_note() {
    let mut s = sine_synth(&[(synth::params::GLIDE, 300.0), (synth::params::RELEASE, 5.0)]);
    let [l, _] = render(
        &mut s,
        &[
            (0, on(57, 1, 1.0)),
            (9_000, off(57, 1)),
            (9_600, on(69, 2, 1.0)),
        ],
        48_000,
    );
    let early = frequency(&l[10_000..11_500]);
    assert!(early > 225.0 && early < 400.0, "gliding: {early}");
    let late = frequency(&l[36_000..]);
    assert!((late / 440.0 - 1.0).abs() < 2e-3, "arrived: {late}");
    // No glide: straight to pitch.
    let mut s = sine_synth(&[(synth::params::RELEASE, 5.0)]);
    let [l, _] = render(
        &mut s,
        &[
            (0, on(57, 1, 1.0)),
            (9_000, off(57, 1)),
            (9_600, on(69, 2, 1.0)),
        ],
        24_000,
    );
    assert!((frequency(&l[10_000..11_500]) / 440.0 - 1.0).abs() < 2e-3);
}

#[test]
fn synth_velocity_sensitivity() {
    let level = |sens: f64, vel: f32| {
        let mut s = sine_synth(&[(synth::params::VELOCITY, sens)]);
        let [l, _] = render(&mut s, &[(0, on(69, 1, vel))], 12_000);
        rms(&l[4_800..])
    };
    // 100 % (default): level follows velocity, as before.
    let r = level(100.0, 0.25) / level(100.0, 1.0);
    assert!((r - 0.25).abs() < 0.01, "{r}");
    // 0 %: every velocity at full level.
    let r = level(0.0, 0.25) / level(0.0, 1.0);
    assert!((r - 1.0).abs() < 0.01, "{r}");
}

#[test]
fn synth_envelope_and_key_tracking_open_the_filter() {
    let saw = |params: &[(ParamId, f64)], key: u8| {
        let mut s = Synth::new();
        s.set_param(synth::params::WAVEFORM, 1.0);
        s.set_param(synth::params::CUTOFF, 300.0);
        s.set_param(synth::params::RESONANCE, 0.0);
        s.set_param(synth::params::SUSTAIN, 100.0);
        for &(id, v) in params {
            s.set_param(id, v);
        }
        let [l, _] = render(&mut s, &[(0, on(key, 1, 1.0))], 12_000);
        brightness(&l[4_800..])
    };
    let closed = saw(&[], 48);
    let env = saw(&[(synth::params::FILTER_ENV, 80.0)], 48);
    assert!(env > 2.0 * closed, "env {env} vs {closed}");
    let down = saw(&[(synth::params::FILTER_ENV, -80.0)], 48);
    assert!(down < closed, "negative env {down} vs {closed}");
    // Key tracking: the cutoff follows the note (100 %: an octave per octave around C4),
    // so a high note above the cutoff comes through instead of being filtered away.
    let level = |params: &[(ParamId, f64)], key: u8| {
        let mut s = Synth::new();
        s.set_param(synth::params::WAVEFORM, 0.0);
        s.set_param(synth::params::CUTOFF, 400.0);
        s.set_param(synth::params::SUSTAIN, 100.0);
        for &(id, v) in params {
            s.set_param(id, v);
        }
        let [l, _] = render(&mut s, &[(0, on(key, 1, 1.0))], 12_000);
        rms(&l[4_800..])
    };
    let tracked = [(synth::params::KEY_TRACKING, 100.0)];
    let high = level(&tracked, 84) / level(&[], 84);
    assert!(high > 3.0, "C6 opens up: x{high}");
    let middle = level(&tracked, 60) / level(&[], 60);
    assert!((middle - 1.0).abs() < 0.01, "C4 unchanged: x{middle}");
}

// --- sampler ----------------------------------------------------------------------------

/// Mono ramp 0 → 1 over `frames` (position readable from the value).
struct Ramp(usize);

impl AudioSource for Ramp {
    fn channels(&self) -> u16 {
        1
    }
    fn frames(&self) -> u64 {
        self.0 as u64
    }
    fn read(&self, _channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (k, o) in out.iter_mut().enumerate() {
            let i = start as usize + k;
            *o = if i < self.0 {
                i as f32 / self.0 as f32
            } else {
                0.0
            };
        }
        true
    }
}

/// Mono sine at `hz`, `frames` long.
struct Sine(f32, usize);

impl AudioSource for Sine {
    fn channels(&self) -> u16 {
        1
    }
    fn frames(&self) -> u64 {
        self.1 as u64
    }
    fn read(&self, _channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (k, o) in out.iter_mut().enumerate() {
            let i = start as usize + k;
            *o = if i < self.1 {
                (std::f32::consts::TAU * self.0 * i as f32 / SR).sin() * 0.5
            } else {
                0.0
            };
        }
        true
    }
}

fn sampler(src: impl AudioSource + 'static, params: &[(ParamId, f64)]) -> Sampler {
    let mut s = Sampler::new(Some(Arc::new(src)));
    s.set_param(sampler::params::ATTACK, 0.0);
    for &(id, v) in params {
        s.set_param(id, v);
    }
    s
}

/// Root key 60 played at 60: the ramp plays at rate 1.
fn held(s: &mut Sampler, frames: usize) -> Vec<f32> {
    let [l, _] = render(s, &[(0, on(60, 1, 1.0))], frames);
    l
}

#[test]
fn sampler_reverse_plays_backwards_and_stops_at_start() {
    let mut s = sampler(Ramp(10_000), &[(sampler::params::REVERSE, 1.0)]);
    let l = held(&mut s, 14_000);
    assert!(l[10] > 0.99, "starts at the end: {}", l[10]);
    assert!(l[5_000] > 0.49 && l[5_000] < 0.51, "{}", l[5_000]);
    assert!(
        l[100..9_900].windows(2).all(|w| w[1] <= w[0] + 1e-6),
        "monotonic down"
    );
    assert!(l[10_100..].iter().all(|v| *v == 0.0), "ended");
    // With a range: End → Start.
    let mut s = sampler(
        Ramp(10_000),
        &[
            (sampler::params::REVERSE, 1.0),
            (sampler::params::START, 20.0),
            (sampler::params::END, 60.0),
        ],
    );
    let l = held(&mut s, 6_000);
    assert!((l[1] - 0.6).abs() < 0.01, "{}", l[1]);
    assert!(l[3_900] > 0.199 && l[3_900] < 0.22, "{}", l[3_900]);
    assert!(l[4_100..].iter().all(|v| *v == 0.0));
}

#[test]
fn sampler_forward_loop_repeats_the_region_while_held() {
    let mut s = sampler(
        Ramp(10_000),
        &[
            (sampler::params::LOOP, 1.0),
            (sampler::params::LOOP_START, 25.0),
            (sampler::params::LOOP_END, 75.0),
        ],
    );
    let l = held(&mut s, 40_000);
    // Beyond the sample's length it still plays, inside the loop's values.
    assert!(l[7_400] > 0.73);
    for (i, v) in l.iter().enumerate().skip(8_000) {
        assert!((0.249..0.751).contains(v), "{i}: {v}");
    }
    // Period = loop length (5 000 frames at rate 1).
    assert!((l[20_000] - l[25_000]).abs() < 1e-4);
    // Released: the loop keeps going through the release, then the voice ends.
    let mut s = sampler(
        Ramp(10_000),
        &[
            (sampler::params::LOOP, 1.0),
            (sampler::params::LOOP_START, 25.0),
            (sampler::params::LOOP_END, 75.0),
            (sampler::params::RELEASE, 50.0),
        ],
    );
    let [l, _] = render(&mut s, &[(0, on(60, 1, 1.0)), (20_000, off(60, 1))], 40_000);
    assert!(rms(&l[20_000..20_500]) > 0.1);
    assert!(l[36_000..].iter().all(|v| *v == 0.0), "released and done");
}

#[test]
fn sampler_ping_pong_turns_at_the_loop_points() {
    let mut s = sampler(
        Ramp(10_000),
        &[
            (sampler::params::LOOP, 2.0),
            (sampler::params::LOOP_START, 40.0),
            (sampler::params::LOOP_END, 60.0),
        ],
    );
    let l = held(&mut s, 30_000);
    for v in &l[6_100..] {
        assert!((0.399..0.601).contains(v), "{v}");
    }
    // Continuous (no wrap jump): the largest step is one frame of the ramp.
    assert!(max_step(&l[6_100..]) < 2.5e-4, "{}", max_step(&l[6_100..]));
    // It goes both ways.
    let ups = l[6_100..].windows(2).filter(|w| w[1] > w[0]).count();
    let downs = l[6_100..].windows(2).filter(|w| w[1] < w[0]).count();
    assert!(ups > 5_000 && downs > 5_000, "{ups} up, {downs} down");
}

#[test]
fn sampler_crossfade_smooths_the_loop_point() {
    let jump = |xfade: f64| {
        let mut s = sampler(
            Sine(331.0, 48_000),
            &[
                (sampler::params::LOOP, 1.0),
                (sampler::params::LOOP_START, 20.0),
                (sampler::params::LOOP_END, 50.0),
                (sampler::params::CROSSFADE, xfade),
            ],
        );
        let l = held(&mut s, 96_000);
        max_step(&l[30_000..])
    };
    // 331 Hz over a 14 400-frame loop: not periodic, so a hard wrap clicks.
    let hard = jump(0.0);
    let soft = jump(30.0);
    let sine_step = std::f32::consts::TAU * 331.0 / SR * 0.5;
    assert!(hard > 3.0 * sine_step, "hard {hard}");
    assert!(soft < 1.2 * sine_step, "soft {soft} vs {sine_step}");
}

#[test]
fn sampler_reverse_loop_and_one_shot_ignore_loop() {
    // Reverse + forward loop: down from the end into the loop, then around it backwards.
    let mut s = sampler(
        Ramp(10_000),
        &[
            (sampler::params::REVERSE, 1.0),
            (sampler::params::LOOP, 1.0),
            (sampler::params::LOOP_START, 30.0),
            (sampler::params::LOOP_END, 50.0),
        ],
    );
    let l = held(&mut s, 30_000);
    for v in &l[8_000..] {
        assert!((0.299..0.501).contains(v), "{v}");
    }
    // One-shot never loops.
    let mut s = sampler(
        Ramp(10_000),
        &[
            (sampler::params::MODE, 0.0),
            (sampler::params::LOOP, 1.0),
            (sampler::params::LOOP_START, 30.0),
            (sampler::params::LOOP_END, 50.0),
        ],
    );
    let l = held(&mut s, 14_000);
    assert!(l[11_000..].iter().all(|v| *v == 0.0));
}

#[test]
fn sampler_adsr_fine_velocity_and_voices() {
    // Sustain 50 %: the held level settles at half of the full-sustain one.
    let settled = |sustain: f64| {
        let mut s = sampler(
            Sine(440.0, 48_000),
            &[
                (sampler::params::DECAY, 50.0),
                (sampler::params::SUSTAIN, sustain),
            ],
        );
        rms(&held(&mut s, 30_000)[20_000..])
    };
    let r = settled(50.0) / settled(100.0);
    assert!((r - 0.5).abs() < 0.01, "{r}");
    // Fine +100 cents: a semitone up.
    let mut s = sampler(Sine(440.0, 48_000), &[(sampler::params::FINE, 100.0)]);
    let f = frequency(&held(&mut s, 20_000)[1_000..]);
    let want = 440.0 * (1.0f32 / 12.0).exp2();
    assert!((f / want - 1.0).abs() < 2e-3, "{f}");
    // Velocity 0 %: soft notes at full level.
    let level = |sens: f64| {
        let mut s = sampler(Sine(440.0, 48_000), &[(sampler::params::VELOCITY, sens)]);
        let [l, _] = render(&mut s, &[(0, on(60, 1, 0.25))], 4_000);
        rms(&l[500..])
    };
    assert!((level(0.0) / level(100.0) - 4.0).abs() < 0.05);
    // Voices 2: a third note steals the oldest.
    let mut s = sampler(Sine(440.0, 48_000), &[(sampler::params::VOICES, 2.0)]);
    let [one, _] = render(&mut s, &[(0, on(60, 1, 1.0))], 4_000);
    let mut s = sampler(Sine(440.0, 48_000), &[(sampler::params::VOICES, 2.0)]);
    let [three, _] = render(
        &mut s,
        &[
            (0, on(60, 1, 1.0)),
            (0, on(60, 2, 1.0)),
            (0, on(60, 3, 1.0)),
        ],
        4_000,
    );
    // Same phase: two voices = twice one.
    let r = rms(&three[1_000..]) / rms(&one[1_000..]);
    assert!((r - 2.0).abs() < 0.05, "{r}");
}

#[test]
fn sampler_filter_darkens_and_off_is_transparent() {
    let bright = |ty: f64, cutoff: f64| {
        let mut s = sampler(
            Sine(5_000.0, 48_000),
            &[
                (sampler::params::FILTER_TYPE, ty),
                (sampler::params::CUTOFF, cutoff),
            ],
        );
        rms(&held(&mut s, 12_000)[2_000..])
    };
    let off = bright(0.0, 20_000.0);
    assert!(bright(1.0, 300.0) < 0.05 * off, "low-pass");
    assert!(bright(2.0, 300.0) > 0.9 * off, "high-pass passes highs");
    assert!(bright(2.0, 15_000.0) < 0.5 * off, "high-pass cuts lows");
    assert!(bright(3.0, 5_000.0) > 0.7 * off, "band-pass at the tone");
}

#[test]
fn sampler_loop_extremes_stay_finite() {
    for (ls, le, mode, xf, rev) in [
        (0.0, 100.0, 1.0, 100.0, 0.0),
        (49.99, 50.0, 1.0, 50.0, 0.0),
        (10.0, 10.02, 2.0, 0.0, 1.0),
        (90.0, 10.0, 1.0, 50.0, 1.0),
        (0.0, 0.5, 1.0, 100.0, 1.0),
    ] {
        let mut s = sampler(
            Ramp(10_000),
            &[
                (sampler::params::LOOP, mode),
                (sampler::params::LOOP_START, ls),
                (sampler::params::LOOP_END, le),
                (sampler::params::CROSSFADE, xf),
                (sampler::params::REVERSE, rev),
                (sampler::params::TRANSPOSE, 24.0),
            ],
        );
        let [l, _] = render(&mut s, &[(0, on(72, 1, 1.0)), (3, on(30, 2, 1.0))], 30_000);
        // Two voices of a 0..1 ramp; band-limited reads ring (Gibbs, under 9 %) where a
        // loop wraps the ramp's 1 -> 0 edge.
        assert!(
            l.iter().all(|v| v.is_finite() && v.abs() <= 2.2),
            "{ls} {le} {mode}"
        );
    }
}
