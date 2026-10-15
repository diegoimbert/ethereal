//! Sound quality of the built-in devices (#520): objective, render-based measurements.
//!
//! `audit` (ignored; run it in release:
//! `cargo test --release -p ether-devices --test sound_quality -- --ignored --nocapture`)
//! prints a markdown table of every instrument and audio effect: levels and DC at the
//! defaults, oscillator aliasing, envelope and voice-steal clicks, zipper noise on every
//! continuous param, filter resonance, compressor / limiter accuracy, saturation aliasing,
//! reverb density and tail smoothness, CPU and denormals. The regression tests below pin
//! the fixes made from it.

mod quality_rig;

use ether_core::protocol::devices::{DeviceCategory, ParamScale};
use ether_core::protocol::model::BuiltinDeviceType as T;
use quality_rig::*;

// --- the audit ----------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Verdict {
    Ok,
    Minor,
    Moderate,
    Severe,
}

struct Row {
    device: String,
    check: String,
    value: String,
    verdict: Verdict,
}

#[derive(Default)]
struct Report(Vec<Row>);

impl Report {
    fn add(&mut self, device: &str, check: impl Into<String>, value: String, verdict: Verdict) {
        let row = Row {
            device: device.to_owned(),
            check: check.into(),
            value,
            verdict,
        };
        eprintln!(
            "{:<22} {:<48} {:<28} {:?}",
            row.device, row.check, row.value, row.verdict
        );
        self.0.push(row);
    }

    /// Lower-is-better metric with `(minor, moderate, severe)` thresholds.
    fn grade(
        &mut self,
        device: &str,
        check: impl Into<String>,
        v: f64,
        unit: &str,
        th: (f64, f64, f64),
    ) {
        let verdict = if v >= th.2 {
            Verdict::Severe
        } else if v >= th.1 {
            Verdict::Moderate
        } else if v >= th.0 {
            Verdict::Minor
        } else {
            Verdict::Ok
        };
        self.add(device, check, format!("{v:.1} {unit}"), verdict);
    }
}

fn hz(key: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0)
}

/// How each instrument is driven.
struct Inst {
    ty: T,
    name: &'static str,
    /// Params for a plain sine voice (click / zipper tests) and a raw saw (aliasing).
    sine: Vec<(u32, f64)>,
    saw: Vec<(u32, f64)>,
    square: Option<Vec<(u32, f64)>>,
    volume: Option<(u32, f64)>,
    cutoff: Option<u32>,
    resonance: Option<(u32, f64)>,
    voices: Option<u32>,
    mode: Option<u32>,
    attack: u32,
}

fn instruments() -> Vec<Inst> {
    vec![
        Inst {
            ty: T::Synth,
            name: "synth",
            sine: vec![(0, 0.0)],
            saw: vec![(0, 1.0), (6, 20000.0), (7, 0.0)],
            square: Some(vec![(0, 2.0), (6, 20000.0), (7, 0.0)]),
            volume: Some((8, -30.0)),
            cutoff: Some(6),
            resonance: Some((7, 100.0)),
            voices: None,
            mode: Some(14),
            attack: 2,
        },
        Inst {
            ty: T::PolySynth,
            name: "poly synth",
            sine: vec![(0, 3.0)],
            saw: vec![(0, 0.0), (26, 20000.0), (27, 0.0)],
            square: Some(vec![(0, 1.0), (26, 20000.0), (27, 0.0)]),
            volume: Some((57, -30.0)),
            cutoff: Some(26),
            resonance: Some((27, 100.0)),
            voices: Some(21),
            mode: Some(22),
            attack: 31,
        },
        Inst {
            ty: T::FmSynth,
            name: "fm synth",
            sine: vec![],
            // Operator A alone with full self-feedback: a saw-like spectrum.
            saw: vec![(12, 100.0)],
            square: None,
            volume: Some((73, -30.0)),
            cutoff: Some(65),
            resonance: Some((66, 100.0)),
            voices: Some(55),
            mode: Some(76),
            attack: 7,
        },
        Inst {
            ty: T::Sampler,
            name: "sampler",
            sine: vec![],
            saw: vec![],
            square: None,
            volume: Some((5, -24.0)),
            cutoff: Some(18),
            resonance: Some((19, 100.0)),
            voices: Some(20),
            mode: None,
            attack: 3,
        },
        Inst {
            ty: T::MultiSampler,
            name: "multisampler",
            sine: vec![],
            saw: vec![],
            square: None,
            volume: Some((0, -30.0)),
            cutoff: Some(11),
            resonance: Some((12, 100.0)),
            voices: Some(3),
            mode: None,
            attack: 6,
        },
    ]
}

fn inst_make(i: &Inst, sine: bool, extra: &[(u32, f64)]) -> Box<dyn ether_core::Device> {
    let mut params = if sine { i.sine.clone() } else { i.saw.clone() };
    if i.ty == T::Sampler && !sine {
        params.push((0, 1.0));
    }
    if i.ty == T::MultiSampler || i.ty == T::Sampler {
        // Open the filter for filter tests (multisampler: low-pass 24, sampler: low-pass).
        if extra.iter().any(|(p, _)| Some(*p) == i.cutoff) {
            params.push((if i.ty == T::Sampler { 17 } else { 10 }, 1.0));
        }
    }
    if i.ty == T::FmSynth && extra.iter().any(|(p, _)| Some(*p) == i.cutoff) {
        params.push((63, 1.0));
    }
    params.extend_from_slice(extra);
    let sample = if sine { SINE_SAMPLE } else { SAW_SAMPLE };
    make_with(i.ty, sample, &params)
}

/// Mono mode (if the instrument has it), a sine voice: the click (>2 kHz burst) of a new
/// key (A3 -> E4) played while the first is held.
fn mono_new_note_click_db(i: &Inst) -> Option<f64> {
    let mode = i.mode?;
    let steal = [
        (secs(0.1), on(57, 0.8)),
        (secs(0.6), on(64, 0.8)),
        (secs(1.1), off(57)),
        (secs(1.1), off(64)),
    ];
    let mut d = inst_make(i, true, &[(mode, 1.0)]);
    let out = render(&mut *d, &silence(), &steal, secs(1.5)).mono();
    let reference = power(&out[secs(0.4)..secs(0.55)]);
    Some(click_db(&out, secs(0.55), secs(0.8), 2000.0, reference))
}

/// A held sine through the instrument's filter: the zipper (>2 kHz burst) of a cutoff
/// jump from 8 kHz to 200 Hz.
fn cutoff_jump_zipper_db(i: &Inst) -> Option<f64> {
    let cut = i.cutoff?;
    let mut d = inst_make(i, true, &[(cut, 8000.0)]);
    let ev = [(0, on(57, 0.8)), (secs(0.5), param(cut, 200.0))];
    let out = render(&mut *d, &silence(), &ev, secs(1.0)).mono();
    let reference = power(&out[secs(0.3)..secs(0.45)]);
    Some(click_db(&out, secs(0.45), secs(0.7), 2000.0, reference))
}

fn audit_instrument(r: &mut Report, i: &Inst) {
    let name = i.name;
    // Levels at the defaults: one note and a four-note chord at velocity 100.
    let vel = 100.0 / 127.0;
    let mut d = make_with(i.ty, SAW_SAMPLE, &[]);
    let one = render(
        &mut *d,
        &silence(),
        &[(0, on(60, vel)), (secs(1.0), off(60))],
        secs(1.5),
    );
    let m = one.mono();
    r.grade(
        name,
        "default note: peak",
        db(peak(&m)),
        "dBFS",
        (-1.0, 0.0, 3.0),
    );
    r.add(
        name,
        "default note: rms (0.3-1 s)",
        format!("{:.1} dBFS", db(rms(&m[secs(0.3)..secs(1.0)]))),
        Verdict::Ok,
    );
    let dc = mean(&m[secs(0.3)..secs(1.0)]).abs() / rms(&m[secs(0.3)..secs(1.0)]).max(1e-9);
    r.grade(
        name,
        "default note: dc / rms",
        db(dc),
        "dB",
        (-50.0, -40.0, -20.0),
    );
    let mut d = make_with(i.ty, SAW_SAMPLE, &[]);
    let chord: Vec<_> = [48u8, 60, 64, 67, 71]
        .iter()
        .map(|&k| (0, on(k, vel)))
        .collect();
    let c = render(&mut *d, &silence(), &chord, secs(1.0));
    r.grade(
        name,
        "default 5-note chord: peak",
        db(peak(&c.mono())),
        "dBFS",
        (-1.0, 0.0, 3.0),
    );

    // Aliasing of the raw oscillator (filter open) at high notes.
    let mut shapes = vec![("saw", i.saw.clone())];
    if let Some(sq) = &i.square {
        shapes.push(("square", sq.clone()));
    }
    for (shape, _) in &shapes {
        for key in [84u8, 96, 108] {
            let mut d = if *shape == "saw" {
                inst_make(i, false, &[])
            } else {
                let mut p = i.square.clone().unwrap();
                p.push((0, p[0].1));
                make_with(i.ty, SAW_SAMPLE, &p)
            };
            let out = render(&mut *d, &silence(), &[(0, on(key, 1.0))], secs(0.8));
            let m = out.mono();
            // From 0.1 s: four octaves up, a sampler's 8 s sample lasts 0.5 s.
            let a = alias_db(&m[secs(0.1)..secs(0.1) + 16384], hz(key), 18_000.0);
            r.grade(
                name,
                format!("{shape} aliasing, key {key} ({:.0} Hz)", hz(key)),
                a,
                "dB",
                (-70.0, -50.0, -35.0),
            );
        }
    }

    // Clicks on a sine voice: onset, retrigger of a sounding key, release.
    let mut d = inst_make(i, true, &[]);
    let ev = [
        (secs(0.1), on(57, 0.8)),
        (secs(0.6), off(57)),
        (secs(0.6) + 1, on(57, 0.8)),
        (secs(1.1), off(57)),
    ];
    let out = render(&mut *d, &silence(), &ev, secs(2.0)).mono();
    let reference = power(&out[secs(0.4)..secs(0.55)]);
    r.grade(
        name,
        "sine: note-on click (>2 kHz burst)",
        click_db(&out, secs(0.05), secs(0.3), 2000.0, reference),
        "dB",
        (-60.0, -45.0, -30.0),
    );
    r.grade(
        name,
        "sine: retrigger click",
        click_db(&out, secs(0.55), secs(0.8), 2000.0, reference),
        "dB",
        (-60.0, -45.0, -30.0),
    );
    r.grade(
        name,
        "sine: release / voice-end click",
        click_db(&out, secs(1.05), secs(2.0), 2000.0, reference),
        "dB",
        (-60.0, -45.0, -30.0),
    );
    let mut d = inst_make(i, true, &[(i.attack, 0.0)]);
    let out = render(&mut *d, &silence(), &ev[..1], secs(0.5)).mono();
    let reference = power(&out[secs(0.3)..secs(0.45)]);
    r.add(
        name,
        "sine: note-on click at attack 0 (ref)",
        format!(
            "{:.1} dB",
            click_db(&out, secs(0.05), secs(0.3), 2000.0, reference)
        ),
        Verdict::Ok,
    );

    // Voice stealing: mono mode (a new key while one sounds), and a single poly voice.
    let steal = [
        (secs(0.1), on(57, 0.8)),
        (secs(0.6), on(64, 0.8)),
        (secs(1.1), off(57)),
        (secs(1.1), off(64)),
    ];
    if let Some(click) = mono_new_note_click_db(i) {
        r.grade(
            name,
            "sine, mono: new-note click",
            click,
            "dB",
            (-60.0, -45.0, -30.0),
        );
    }
    if let Some(voices) = i.voices {
        let mut d = inst_make(i, true, &[(voices, 1.0)]);
        let out = render(&mut *d, &silence(), &steal, secs(1.5)).mono();
        let reference = power(&out[secs(0.4)..secs(0.55)]);
        r.grade(
            name,
            "sine, 1 voice: steal click",
            click_db(&out, secs(0.55), secs(0.8), 2000.0, reference),
            "dB",
            (-60.0, -45.0, -30.0),
        );
    }

    // Zipper noise: volume and cutoff jumps on a held sine.
    if let Some((vol, to)) = i.volume {
        let mut d = inst_make(i, true, &[]);
        let ev = [(0, on(57, 0.8)), (secs(0.5), param(vol, to))];
        let out = render(&mut *d, &silence(), &ev, secs(1.0)).mono();
        let reference = power(&out[secs(0.3)..secs(0.45)]);
        r.grade(
            name,
            "volume jump: zipper (>2 kHz burst)",
            click_db(&out, secs(0.45), secs(0.7), 2000.0, reference),
            "dB",
            (-60.0, -45.0, -30.0),
        );
    }
    if let Some(zipper) = cutoff_jump_zipper_db(i) {
        r.grade(
            name,
            "cutoff jump 8k->200: zipper",
            zipper,
            "dB",
            (-60.0, -45.0, -30.0),
        );
    }

    // Resonance: boost at max resonance and stability under a fast cutoff sweep.
    if let (Some(cut), Some((res, max))) = (i.cutoff, i.resonance) {
        let level = |res_v: f64| {
            let mut d = inst_make(i, false, &[(cut, 1000.0), (res, res_v)]);
            let out = render(&mut *d, &silence(), &[(0, on(36, 1.0))], secs(0.8)).mono();
            (peak(&out[secs(0.3)..]), rms(&out[secs(0.3)..]))
        };
        let (p0, r0) = level(0.0);
        let (p1, r1) = level(max);
        r.add(
            name,
            "max resonance @1 kHz, C2 saw: rms / peak boost",
            format!("{:+.1} / {:+.1} dB", db(r1 / r0), db(p1 / p0)),
            if db(p1 / p0) > 18.0 {
                Verdict::Moderate
            } else {
                Verdict::Ok
            },
        );
        let mut d = inst_make(i, false, &[(cut, 100.0), (res, max)]);
        let mut ev = vec![(0, on(36, 1.0))];
        for b in 0..(S / BLOCK) {
            let f = 100.0 * 100f64.powf(b as f64 / (S / BLOCK) as f64);
            ev.push((b * BLOCK, param(cut, f)));
        }
        let out = render(&mut *d, &silence(), &ev, S).mono();
        let ok = finite(&out) && peak(&out) < 4.0;
        r.add(
            name,
            "max resonance, 100 Hz->10 kHz sweep in 1 s",
            format!(
                "peak {:.1} dBFS{}",
                db(peak(&out)),
                if finite(&out) { "" } else { ", non-finite" }
            ),
            if ok { Verdict::Ok } else { Verdict::Severe },
        );
    }

    // CPU (8 voices held 2 s, released) and the silent tail.
    let chord: Vec<_> = (0..8u8)
        .map(|k| (0, on(48 + k * 3, 0.8)))
        .chain((0..8u8).map(|k| (secs(2.0), off(48 + k * 3))))
        .collect();
    let mut d = make_with(i.ty, SAW_SAMPLE, &[]);
    let out = render(&mut *d, &silence(), &chord, secs(8.0));
    let active = out.cpu(0.1, 2.0) * 100.0;
    let tail = out.cpu(6.0, 8.0) * 100.0;
    r.grade(
        name,
        "cpu, 8 voices (default patch)",
        active,
        "% of 1 core",
        (5.0, 15.0, 40.0),
    );
    r.add(
        name,
        "cpu, silent tail after release",
        format!("{tail:.2} %"),
        if tail > active * 0.25 {
            Verdict::Minor
        } else {
            Verdict::Ok
        },
    );
    r.add(
        name,
        "subnormal output samples",
        format!(
            "{}",
            has_subnormals(&out.out[0]) || has_subnormals(&out.out[1])
        ),
        if has_subnormals(&out.out[0]) {
            Verdict::Minor
        } else {
            Verdict::Ok
        },
    );
    if i.ty == T::PolySynth {
        let mut d = make_with(i.ty, SAW_SAMPLE, &[(61, 8.0)]);
        let out = render(&mut *d, &silence(), &chord, secs(2.5));
        r.grade(
            name,
            "cpu, 8 voices x 8 unison",
            out.cpu(0.1, 2.0) * 100.0,
            "% of 1 core",
            (10.0, 25.0, 50.0),
        );
    }
}

fn audit_effect(r: &mut Report, ty: T) {
    let name = &format!("{ty:?}").to_lowercase();
    let input = program(4.0);
    let mut d = make(ty, &[]);
    let lat = d.latency();
    let out = render(&mut *d, &input, &[], secs(4.0));
    let (inl, outl) = (&input[0], &out.out[0]);
    let ok = finite(outl) && finite(&out.out[1]);
    r.add(
        name,
        "defaults on a mix: finite",
        format!("{ok}"),
        if ok { Verdict::Ok } else { Verdict::Severe },
    );
    r.add(
        name,
        "defaults on a mix (peak -2 dBFS): peak out",
        format!("{:.1} dBFS", db(peak(outl).max(peak(&out.out[1])))),
        if peak(outl) > 1.0 {
            Verdict::Moderate
        } else {
            Verdict::Ok
        },
    );
    r.add(
        name,
        "defaults: rms change",
        format!("{:+.1} dB", db(rms(outl) / rms(inl))),
        Verdict::Ok,
    );
    let dc = mean(&outl[secs(1.0)..]).abs() / rms(&outl[secs(1.0)..]).max(1e-9);
    r.grade(
        name,
        "defaults: dc / rms",
        db(dc),
        "dB",
        (-50.0, -40.0, -20.0),
    );
    let shift = lat as usize;
    let diff: Vec<f32> = outl[shift..]
        .iter()
        .zip(inl.iter())
        .map(|(o, i)| o - i)
        .collect();
    r.add(
        name,
        "defaults: change vs dry (rms diff)",
        format!("{:.1} dB, latency {lat}", db(rms(&diff) / rms(inl))),
        Verdict::Ok,
    );

    // CPU on the mix and on the silent input after it.
    let mut d = make(ty, &[]);
    let out = render(&mut *d, &input, &[], secs(10.0));
    let active = out.cpu(0.0, 4.0) * 100.0;
    let quiet = out.cpu(8.0, 10.0) * 100.0;
    r.grade(
        name,
        "cpu on a mix",
        active,
        "% of 1 core",
        (2.0, 8.0, 25.0),
    );
    r.add(
        name,
        "cpu, silent input after (s 8-10)",
        format!("{quiet:.2} %"),
        if quiet > active * 0.5 && active > 0.2 {
            Verdict::Minor
        } else {
            Verdict::Ok
        },
    );
    let sub = has_subnormals(&out.out[0]) || has_subnormals(&out.out[1]);
    r.add(
        name,
        "subnormal output samples",
        format!("{sub}"),
        if sub { Verdict::Minor } else { Verdict::Ok },
    );

    for z in zippers(ty) {
        if z.burst > -70.0 && z.burst - z.steady > 6.0 {
            r.grade(
                name,
                format!("zipper: {} {} -> {:.3}", z.param, z.from, z.to),
                z.burst,
                &format!("dB (steady {:.0})", z.steady),
                (-70.0, -55.0, -40.0),
            );
        }
    }
}

struct Zipper {
    param: String,
    from: f64,
    to: f64,
    /// Energy above 3 kHz (dB, relative to the signal) in the 200 ms after the jump and in
    /// the steady state before / after it.
    burst: f64,
    steady: f64,
}

/// Every continuous automatable param of `ty` jumps (default -> towards its far end, at
/// 0.5 s) under a 220 Hz sine: how much broadband energy the jump adds (a saturator's own
/// harmonics are steady, not zipper noise).
fn zippers(ty: T) -> Vec<Zipper> {
    let desc = ether_devices::descriptor(ty);
    let tone = stereo(sine(220.0, 0.25, secs(1.0)));
    let mut out = Vec::new();
    for p in desc
        .params
        .iter()
        .filter(|p| p.automatable && !p.hidden && p.labels.is_none() && p.step.is_none())
    {
        let to = if p.default - p.min < p.max - p.default {
            match p.scale {
                ParamScale::Log => (p.default * 8.0).min(p.max),
                _ => p.default + (p.max - p.default) * 0.75,
            }
        } else {
            p.min + (p.default - p.min) * 0.25
        };
        let mut d = make(ty, &[]);
        let x = render(&mut *d, &tone, &[(secs(0.5), param(p.id.0, to))], secs(1.0)).mono();
        let reference = power(&x[secs(0.3)..secs(0.45)])
            .max(power(&x[secs(0.85)..secs(1.0)]))
            .max(1e-12);
        let burst = click_db(&x, secs(0.48), secs(0.68), 3000.0, reference);
        let steady = click_db(&x, secs(0.3), secs(0.45), 3000.0, reference).max(click_db(
            &x,
            secs(0.85),
            secs(1.0),
            3000.0,
            reference,
        ));
        out.push(Zipper {
            param: p.name.to_lowercase(),
            from: p.default,
            to,
            burst,
            steady,
        });
    }
    out
}

/// Largest error (dB) of a compressor's static curve against its settings on a 1 kHz sine
/// from -40 to 0 dBFS (compressor: threshold -18, 4:1, 6 dB knee; multiband: the mid band's
/// 3:1 at -20, its 6 dB knee ignored, crossovers at 20 Hz and 16 kHz so that the tone is all
/// in the mid band: the other bands' uncompressed share of it would add to the mid band's),
/// and the `level:got/want` detail.
fn static_curve_error(ty: T) -> (f64, String) {
    let (thr, ratio, params) = if ty == T::Compressor {
        (-18.0, 4.0, vec![])
    } else {
        (-20.0, 3.0, vec![(0, 20.0), (1, 16_000.0)])
    };
    let mut worst: f64 = 0.0;
    let mut detail = String::new();
    for lvl in [-40.0f64, -30.0, -24.0, -18.0, -12.0, -6.0, 0.0] {
        let mut d = make(ty, &params);
        let x = sine(1000.0, 10f64.powf(lvl / 20.0), secs(2.0));
        let out = render(&mut *d, &stereo(x.clone()), &[], secs(2.0));
        let got = db(rms(&out.out[0][secs(1.5)..]) / rms(&x[secs(1.5)..]));
        let over = lvl - thr;
        let knee = if ty == T::Compressor { 6.0 } else { 0.0 };
        let want = if 2.0 * over < -knee {
            0.0
        } else if 2.0 * over > knee {
            over / ratio - over
        } else {
            (1.0 / ratio - 1.0) * (over + knee / 2.0).powi(2) / (2.0 * knee)
        };
        worst = worst.max((got - want).abs());
        detail += &format!("{lvl:.0}:{got:+.1}/{want:+.1} ");
    }
    (worst, detail)
}

/// Aliasing of a transient shaper with `Clip` on and +12 dB of output gain, on a 4987 Hz
/// sine at -6 dBFS.
fn shaper_clip_alias_db() -> f64 {
    let x = sine(4987.0, 0.5, secs(0.8));
    let mut d = make(T::TransientShaper, &[(4, 1.0), (5, 12.0)]);
    let out = render(&mut *d, &stereo(x), &[], secs(0.8)).mono();
    alias_db(&out[secs(0.3)..secs(0.3) + 16384], 4987.0, 18_000.0)
}

fn audit_specials(r: &mut Report) {
    // Compressor: static curve (threshold -18, ratio 4, knee 6) on a 1 kHz sine.
    for ty in [T::Compressor, T::MultibandCompressor] {
        let name = &format!("{ty:?}").to_lowercase();
        let (worst, detail) = static_curve_error(ty);
        r.grade(
            name,
            "static gain curve, sine (max error)",
            worst,
            "dB",
            (1.0, 2.0, 4.0),
        );
        r.add(name, "  curve in:got/want", detail, Verdict::Ok);
    }
    // Multiband compressor below threshold: crossover sum flatness.
    {
        let mut lo: f64 = 0.0;
        let mut hi: f64 = 0.0;
        for f in [
            50.0, 150.0, 200.0, 300.0, 1000.0, 2000.0, 2500.0, 3500.0, 8000.0,
        ] {
            let mut d = make(T::MultibandCompressor, &[]);
            let x = sine(f, 0.01, secs(0.5));
            let out = render(&mut *d, &stereo(x.clone()), &[], secs(0.5));
            let g = db(rms(&out.out[0][secs(0.3)..]) / rms(&x[secs(0.3)..]));
            lo = lo.min(g);
            hi = hi.max(g);
        }
        r.grade(
            "multibandcompressor",
            "below threshold: response ripple 50 Hz-8 kHz",
            hi - lo,
            "dB",
            (0.5, 1.5, 3.0),
        );
    }
    // Limiter: sample and true peak with +12 dB into a -0.3 dB ceiling.
    {
        let mut d = make(T::Limiter, &[(0, 12.0)]);
        let input = program(3.0);
        let out = render(&mut *d, &input, &[], secs(3.0));
        let sp = db(peak(&out.out[0]));
        let tp = db(true_peak(&out.out[0][secs(0.5)..secs(2.0)]));
        r.grade(
            "limiter",
            "+12 dB drive: sample peak over -0.3 ceiling",
            (sp + 0.3).max(0.0),
            "dB",
            (0.01, 0.5, 1.0),
        );
        r.grade(
            "limiter",
            "+12 dB drive: true peak over ceiling",
            (tp + 0.3).max(0.0),
            "dB",
            (0.3, 1.0, 2.0),
        );
    }
    // Saturator aliasing: 4987 Hz sine at -6 dBFS, drive 24 dB, per curve and oversampling.
    let x = sine(4987.0, 0.5, secs(0.8));
    for curve in 0..6 {
        let curve_name = ["soft", "hard", "tube", "tape", "fold", "rectify"][curve];
        for os in 0..3 {
            let mut d = make(
                T::Saturator,
                &[(0, curve as f64), (1, 24.0), (4, os as f64)],
            );
            let out = render(&mut *d, &stereo(x.clone()), &[], secs(0.8)).mono();
            let a = alias_db(&out[secs(0.3)..secs(0.3) + 16384], 4987.0, 18_000.0);
            r.grade(
                "saturator",
                format!(
                    "{curve_name}, drive 24, os {}: aliasing",
                    ["off", "2x", "4x"][os]
                ),
                a,
                "dB",
                (-60.0, -40.0, -25.0),
            );
        }
    }
    {
        let mut d = make(T::Saturator, &[]);
        let out = render(&mut *d, &stereo(x.clone()), &[], secs(0.8)).mono();
        let a = alias_db(&out[secs(0.3)..secs(0.3) + 16384], 4987.0, 18_000.0);
        r.grade(
            "saturator",
            "defaults (soft, 6 dB, 2x): aliasing",
            a,
            "dB",
            (-60.0, -40.0, -25.0),
        );
    }
    for (ty, params, label) in [
        (
            T::AutoFilter,
            vec![(3u32, 24.0), (1, 20000.0)],
            "drive 24, open",
        ),
        (
            T::TransientShaper,
            vec![(4, 1.0), (5, 12.0)],
            "clip on, +12 dB",
        ),
        (
            T::Compressor,
            vec![(2, 0.1), (3, 5.0), (0, -40.0)],
            "fastest attack/release, -40 dB thr",
        ),
        (
            T::Limiter,
            vec![(0, 18.0), (2, 1.0)],
            "+18 dB, release 1 ms",
        ),
    ] {
        let mut d = make(ty, &params);
        let out = render(&mut *d, &stereo(x.clone()), &[], secs(0.8)).mono();
        let a = alias_db(&out[secs(0.3)..secs(0.3) + 16384], 4987.0, 18_000.0);
        r.grade(
            &format!("{ty:?}").to_lowercase(),
            format!("{label}: aliasing (4987 Hz sine)"),
            a,
            "dB",
            (-60.0, -40.0, -25.0),
        );
    }
    // Modulation effects: interpolation noise on a 1 kHz sine (energy above 6 kHz).
    for ty in [T::Chorus, T::Flanger, T::Phaser, T::Delay] {
        let mut d = make(ty, if ty == T::Delay { &[(9, 50.0)] } else { &[] });
        let x = sine(1000.0, 0.5, secs(2.0));
        let out = render(&mut *d, &stereo(x), &[], secs(2.0)).mono();
        let n = hf_db(&out[secs(1.0)..secs(1.0) + 32768], 6000.0);
        r.grade(
            &format!("{ty:?}").to_lowercase(),
            "1 kHz sine: energy above 6 kHz (interp. noise)",
            n,
            "dB",
            (-70.0, -55.0, -40.0),
        );
    }
    // Delay at max feedback: bounded.
    {
        let mut d = make(T::Delay, &[(3, 95.0), (4, 100.0)]);
        let out = render(&mut *d, &program(1.0), &[], secs(12.0));
        let late = peak(&out.out[0][secs(10.0)..]);
        r.add(
            "delay",
            "feedback 95 %: peak 9 s after input",
            format!("{:.1} dBFS", db(late)),
            if late > 1.0 || !finite(&out.out[0]) {
                Verdict::Severe
            } else {
                Verdict::Ok
            },
        );
    }
    // Reverbs: impulse response at 100 % wet.
    // (A damped tail darkens as it decays, and the roughness of a narrower band is higher:
    // the undamped case shows the tank's own smoothness.)
    for (ty, params, label) in [
        (T::Reverb, vec![(5u32, 100.0)], " (defaults, 100 % wet)"),
        (
            T::Reverb,
            vec![(5, 100.0), (2, 8.0), (1, 100.0)],
            " (size 100, decay 8 s)",
        ),
        (
            T::Reverb,
            vec![(5, 100.0), (2, 8.0), (1, 100.0), (3, 0.0)],
            " (size 100, decay 8 s, damping 0)",
        ),
        (
            T::ConvolutionReverb,
            vec![(0, 100.0)],
            " (defaults, 100 % wet)",
        ),
    ] {
        let name = &format!("{ty:?}").to_lowercase();
        let mut d = make(ty, &params);
        let mut imp = vec![0.0f32; 16];
        imp[0] = 1.0;
        let out = render(&mut *d, &stereo(imp), &[], secs(12.0));
        let (l, rr) = (&out.out[0], &out.out[1]);
        let rt = rt60(l);
        r.add(
            name,
            format!("rt60{label}"),
            rt.map_or("n/a".into(), |t| format!("{t:.2} s")),
            Verdict::Ok,
        );
        let onset = l.iter().position(|v| v.abs() > 1e-4).unwrap_or(0) as f64 / f64::from(SR);
        for t in [0.05, 0.1, 0.25] {
            let ned = echo_density(l, onset + t);
            let v = if ned < 0.5 {
                Verdict::Moderate
            } else if ned < 0.8 {
                Verdict::Minor
            } else {
                Verdict::Ok
            };
            r.add(
                name,
                format!("echo density @ {:.0} ms after onset{label}", t * 1000.0),
                format!("{ned:.2}"),
                v,
            );
        }
        r.grade(
            name,
            format!("tail roughness{label}"),
            tail_roughness_db(l, onset + 0.15),
            "dB",
            (1.0, 2.0, 4.0),
        );
        let seg = &l[secs(onset + 0.3)..secs(onset + 0.3) + 32768];
        r.grade(
            name,
            format!("tail ringing (peaks over 1/3-oct){label}"),
            ringing_db(seg),
            "dB",
            (12.0, 16.0, 22.0),
        );
        r.add(
            name,
            format!("tail l/r correlation{label}"),
            format!(
                "{:.2}",
                correlation(
                    &l[secs(onset + 0.1)..secs(onset + 1.0)],
                    &rr[secs(onset + 0.1)..secs(onset + 1.0)]
                )
            ),
            Verdict::Ok,
        );
        let dcv = mean(&l[secs(onset)..secs(onset + 2.0)]).abs()
            / rms(&l[secs(onset)..secs(onset + 2.0)]).max(1e-12);
        r.grade(
            name,
            format!("ir dc / rms{label}"),
            db(dcv),
            "dB",
            (-40.0, -30.0, -20.0),
        );
    }
    // Gate: open/close clicks on a 220 Hz sine burst (0.5 s on, 0.5 s at -60 dB).
    {
        let n = secs(2.0);
        // 20 ms raised-cosine transitions: the input itself doesn't click.
        let fade = secs(0.02) as f64;
        let env = |i: usize| {
            let t = (i % secs(1.0)) as f64;
            let half = secs(0.5) as f64;
            let up = if t < fade {
                0.5 - 0.5 * (std::f64::consts::PI * t / fade).cos()
            } else if t < half {
                1.0
            } else if t < half + fade {
                0.5 + 0.5 * (std::f64::consts::PI * (t - half) / fade).cos()
            } else {
                0.0
            };
            0.001 + 0.999 * up
        };
        let x: Vec<f32> = sine(220.0, 0.25, n)
            .iter()
            .enumerate()
            .map(|(i, v)| (f64::from(*v) * env(i)) as f32)
            .collect();
        let mut d = make(T::Gate, &[]);
        let out = render(&mut *d, &stereo(x), &[], n).mono();
        let reference = power(&out[secs(0.2)..secs(0.4)]);
        r.grade(
            "gate",
            "close click (sine burst, defaults)",
            click_db(&out, secs(0.45), secs(0.8), 2000.0, reference),
            "dB",
            (-60.0, -45.0, -30.0),
        );
        r.grade(
            "gate",
            "open click (sine burst, defaults)",
            click_db(&out, secs(0.95), secs(1.3), 2000.0, reference),
            "dB",
            (-60.0, -45.0, -30.0),
        );
    }
}

#[test]
#[ignore = "audit report: run in release with --nocapture"]
fn audit() {
    let mut r = Report::default();
    for i in instruments() {
        audit_instrument(&mut r, &i);
    }
    for ty in T::ALL {
        let d = ether_devices::descriptor(ty);
        if d.category == DeviceCategory::AudioEffect
            && !matches!(ty, T::ExternalAudioEffect | T::AudioEffectRack)
        {
            audit_effect(&mut r, ty);
        }
    }
    audit_specials(&mut r);
    println!("\n| device | check | value | verdict |\n|---|---|---|---|");
    for row in r.0.iter().filter(|row| row.verdict != Verdict::Ok) {
        println!(
            "| {} | {} | {} | {:?} |",
            row.device, row.check, row.value, row.verdict
        );
    }
}

// --- regression tests ---------------------------------------------------------------------

fn audio_effects() -> Vec<T> {
    T::ALL
        .into_iter()
        .filter(|&ty| {
            ether_devices::descriptor(ty).category == DeviceCategory::AudioEffect
                && !matches!(ty, T::ExternalAudioEffect | T::AudioEffectRack)
        })
        .collect()
}

/// A knob jump on any continuous param of any audio effect adds no audible broadband burst
/// (#520: the v0.2 effects ramped every change over 32 samples, delay-time jumps swept the
/// echoes' pitch by octaves, lookahead changes jumped the audio tap).
#[test]
fn effect_knob_jumps_do_not_zipper() {
    let mut bad = Vec::new();
    for ty in audio_effects() {
        for z in zippers(ty) {
            if z.burst > -60.0 && z.burst - z.steady > 6.0 {
                bad.push(format!(
                    "{ty:?} {} -> {}: {:.1} dB (steady {:.1})",
                    z.param, z.to, z.burst, z.steady
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Aliasing of the sample engines playing a band-limited saw sample (root C4) at `key`.
fn sample_alias_db(ty: T, key: u8) -> f64 {
    let mut d = make_with(ty, SAW_SAMPLE, &[]);
    // From 0.1 s: four octaves up, the 8 s sample lasts 0.5 s.
    let out = render(&mut *d, &silence(), &[(0, on(key, 1.0))], secs(0.5)).mono();
    alias_db(&out[secs(0.1)..secs(0.1) + 16384], hz(key), 18_000.0)
}

/// A sample pitched up 2, 3 and 4 octaves doesn't alias (#520: linear / Hermite reads
/// aliased at -19, -15 and -5 dB).
#[test]
fn pitched_samples_do_not_alias() {
    for ty in [T::Sampler, T::MultiSampler] {
        for key in [84u8, 96, 108] {
            let a = sample_alias_db(ty, key);
            eprintln!("{ty:?} key {key}: {a:.1} dB");
            assert!(a < -70.0, "{ty:?} key {key}: {a:.1} dB");
        }
    }
}

/// The multisampler's 24 dB low-pass at full resonance boosts like one resonant stage, not
/// two (#520: +24 dB, a sweep peaked at +21 dBFS).
#[test]
fn multisampler_resonance_is_bounded() {
    // Low-pass 24 (type 1), cutoff 1 kHz, a C2 saw sample.
    let level = |res: f64| {
        let mut d = make_with(
            T::MultiSampler,
            SAW_SAMPLE,
            &[(10, 1.0), (11, 1000.0), (12, res)],
        );
        let out = render(&mut *d, &silence(), &[(0, on(36, 1.0))], secs(0.8)).mono();
        peak(&out[secs(0.3)..])
    };
    let boost = db(level(100.0) / level(0.0));
    eprintln!("boost {boost:.1} dB");
    assert!(boost < 15.0, "{boost:.1} dB");
    let mut d = make_with(
        T::MultiSampler,
        SAW_SAMPLE,
        &[(10, 1.0), (11, 100.0), (12, 100.0)],
    );
    let mut ev = vec![(0, on(36, 1.0))];
    for b in 0..(S / BLOCK) {
        ev.push((
            b * BLOCK,
            param(11, 100.0 * 100f64.powf(b as f64 / (S / BLOCK) as f64)),
        ));
    }
    let out = render(&mut *d, &silence(), &ev, S).mono();
    eprintln!("sweep peak {:.1} dBFS", db(peak(&out)));
    assert!(finite(&out) && peak(&out) < 1.0, "{}", db(peak(&out)));
}

/// Saturator aliasing at 24 dB of drive on a 4987 Hz sine (#520: 2x aliased at -30 to -35
/// dB, 4x hard clipping at -39 and rectify at -19, before ADAA; the default is now 4x).
#[test]
fn saturator_oversampled_curves_alias_little() {
    let x = sine(4987.0, 0.5, secs(0.8));
    // (curve, oversampling, bound): 2x / 4x of soft, hard, tube, tape; rectify at 4x.
    let cases = [
        (0, 1, -50.0),
        (1, 1, -48.0),
        (2, 1, -50.0),
        (3, 1, -50.0),
        (0, 2, -80.0),
        (1, 2, -60.0),
        (2, 2, -70.0),
        (3, 2, -70.0),
        (5, 2, -40.0),
    ];
    for (curve, os, bound) in cases {
        let mut d = make(
            T::Saturator,
            &[(0, f64::from(curve)), (1, 24.0), (4, f64::from(os))],
        );
        let out = render(&mut *d, &stereo(x.clone()), &[], secs(0.8)).mono();
        let a = alias_db(&out[secs(0.3)..secs(0.3) + 16384], 4987.0, 18_000.0);
        assert!(a < bound, "curve {curve} os {os}: {a:.1} dB");
    }
    // A fresh saturator is the clean one.
    let d = make(T::Saturator, &[]);
    assert_eq!(d.param(ether_core::protocol::model::ParamId(4)), Some(2.0));
}

/// A fresh instrument's chord doesn't clip (#520: the FM synth's five-note chord at velocity
/// 100 peaked at +5.6 dBFS).
#[test]
fn fresh_synth_chords_do_not_clip() {
    for ty in [T::Synth, T::PolySynth, T::FmSynth] {
        let mut d = make(ty, &[]);
        let chord: Vec<_> = [48u8, 60, 64, 67, 71]
            .iter()
            .map(|&k| (0, on(k, 100.0 / 127.0)))
            .collect();
        let out = render(&mut *d, &silence(), &chord, secs(1.0)).mono();
        let p = db(peak(&out));
        assert!(p < -0.5, "{ty:?}: {p:.1} dBFS");
        assert!(p > -12.0, "{ty:?}: too quiet, {p:.1} dBFS");
    }
}

/// A fresh convolution reverb is a reverb (#520: it had no impulse response and passed
/// audio through untouched).
#[test]
fn fresh_convolution_reverb_reverberates() {
    let mut d = make(T::ConvolutionReverb, &[(0, 100.0)]);
    let mut imp = vec![0.0f32; 16];
    imp[0] = 1.0;
    let out = render(&mut *d, &stereo(imp), &[], secs(2.0)).mono();
    // Energy half a second after the impulse: a tail, not a dry click.
    assert!(
        rms(&out[secs(0.5)..secs(1.0)]) > 1e-4,
        "{}",
        rms(&out[secs(0.5)..secs(1.0)])
    );
}

/// Bound of the poly synth's aliasing with the filter open (#845: its filter's saturating
/// input aliased at -48 to -52 dB before it ran 2x oversampled).
const POLY_ALIAS_DB: f64 = -57.0;

/// The virtual-analog oscillators alias little on high notes with the filter open (#520:
/// 2-point polyBLEP left -35 to -42 dB).
#[test]
fn analog_oscillators_alias_little() {
    // (type, params for saw / square with the filter fully open, bound)
    let cases = [
        (T::Synth, &[(0, 1.0), (6, 20000.0), (7, 0.0)], -50.0),
        (T::Synth, &[(0, 2.0), (6, 20000.0), (7, 0.0)], -50.0),
        (
            T::PolySynth,
            &[(0, 0.0), (26, 20000.0), (27, 0.0)],
            POLY_ALIAS_DB,
        ),
        (
            T::PolySynth,
            &[(0, 1.0), (26, 20000.0), (27, 0.0)],
            POLY_ALIAS_DB,
        ),
    ];
    for (ty, params, bound) in cases {
        for key in [84u8, 96, 108] {
            let mut d = make(ty, params);
            let out = render(&mut *d, &silence(), &[(0, on(key, 1.0))], secs(0.5)).mono();
            let a = alias_db(&out[secs(0.1)..secs(0.1) + 16384], hz(key), 18_000.0);
            assert!(a < bound, "{ty:?} {params:?} key {key}: {a:.1} dB");
        }
    }
}

/// The limiter holds true (inter-sample) peaks under the ceiling (#520: sample-peak
/// detection let them 0.8 dB over at +12 dB of drive).
#[test]
fn limiter_true_peaks_stay_under_the_ceiling() {
    let mut d = make(T::Limiter, &[(0, 12.0)]);
    let out = render(&mut *d, &program(1.5), &[], secs(1.5));
    let over = db(true_peak(&out.out[0][secs(0.3)..secs(1.2)])) + 0.3;
    assert!(over < 0.2, "{over:.2} dB over the ceiling");
}

/// A stolen sample voice fades out instead of being cut (#520: a hard cut, -42 dB click).
/// The new notes attack in 10 ms, so a click can only come from the stolen voice.
#[test]
fn stolen_sample_voices_fade_out() {
    let steal = [(secs(0.1), on(57, 0.8)), (secs(0.6), on(64, 0.8))];
    for (ty, voices, attack) in [(T::Sampler, 20, 3), (T::MultiSampler, 3, 6)] {
        let mut d = make_with(ty, SINE_SAMPLE, &[(voices, 1.0), (attack, 10.0)]);
        let out = render(&mut *d, &silence(), &steal, secs(1.0)).mono();
        let reference = power(&out[secs(0.4)..secs(0.55)]);
        let click = click_db(&out, secs(0.55), secs(0.8), 2000.0, reference);
        assert!(click < -55.0, "{ty:?}: {click:.1} dB");
    }
}

/// The auto filter's drive stage, oversampled 4x, folds little back (#520: -17 dB at 1x),
/// and its dry path stays aligned with the oversampled wet one (no comb at 50 % mix).
#[test]
fn auto_filter_drive_aliases_little_and_mix_stays_aligned() {
    let x = sine(4987.0, 0.5, secs(0.8));
    let mut d = make(T::AutoFilter, &[(3, 24.0), (1, 20000.0)]);
    let out = render(&mut *d, &stereo(x.clone()), &[], secs(0.8)).mono();
    let a = alias_db(&out[secs(0.3)..secs(0.3) + 16384], 4987.0, 18_000.0);
    assert!(a < -55.0, "{a:.1} dB");
    // Open low-pass (flat at 1.5 kHz), no drive, 50 % mix: the tone keeps its level (a
    // dry path off by the 29-sample latency would cancel it).
    let tone = sine(1500.0, 0.5, secs(0.5));
    let mut d = make(T::AutoFilter, &[(1, 20000.0), (2, 0.0), (13, 50.0)]);
    let out = render(&mut *d, &stereo(tone.clone()), &[], secs(0.5)).mono();
    let g = db(rms(&out[secs(0.2)..]) / rms(&tone[secs(0.2)..]));
    assert!(g.abs() < 0.3, "1.5 kHz at 50 % mix: {g:.2} dB");
}

/// The `params` of a factory preset (plain values by id).
fn preset_params(json: &str) -> Vec<(u32, f64)> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    v["preset"]["params"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.parse().unwrap(), v.as_f64().unwrap()))
                .collect()
        })
        .unwrap_or_default()
}

/// Every factory preset of the synths and the (non-rack) audio effects sounds sane on a
/// fresh instance (#520): finite, no clipping, audible, and an effect's level within a few
/// dB of its input (presets that deliberately change the level aside).
#[test]
fn factory_presets_render_sanely() {
    let mut bad = Vec::new();
    for ty in [T::Synth, T::PolySynth, T::FmSynth] {
        for p in ether_devices::factory_presets(ty) {
            let mut d = make(ty, &preset_params(p.json));
            let chord: Vec<_> = [48u8, 55, 60, 64]
                .iter()
                .map(|&k| (0, on(k, 100.0 / 127.0)))
                .collect();
            let out = render(&mut *d, &silence(), &chord, secs(1.5)).mono();
            let pk = db(peak(&out));
            if !finite(&out) || pk > 0.0 || pk < -36.0 {
                bad.push(format!("{}: chord peak {pk:.1} dBFS", p.id));
            }
        }
    }
    let input = program(1.5);
    let in_rms = rms(&input[0][secs(0.5)..]);
    for ty in audio_effects().into_iter().chain([T::AudioEffectRack]) {
        for p in ether_devices::factory_presets(ty) {
            let out = if p.json.contains("\"rack\"") {
                render_rack_preset(p.json, &input)
            } else {
                let mut d = make(ty, &preset_params(p.json));
                render(&mut *d, &input, &[], secs(1.5)).out
            };
            let out = Render {
                out,
                block_secs: Vec::new(),
            };
            let o = &out.out[0];
            let pk = db(peak(o).max(peak(&out.out[1])));
            let change = db(rms(&o[secs(0.5)..]) / in_rms);
            // Deliberate level changes: the -6 dB utility and the muting tuner; the wah is a
            // band-pass, whose level depends on the material (this mix is bass-heavy).
            let level_ok = matches!(
                p.id,
                "utility/minus-6-db" | "tuner/silent-tuning" | "auto-filter/envelope-wah"
            ) || change.abs() <= 9.0;
            if !finite(o) || pk > 0.5 || !level_ok {
                bad.push(format!(
                    "{}: peak {pk:.1} dBFS, level {change:+.1} dB",
                    p.id
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// An audio effect rack preset rendered as its chains: each chain's devices in series (at
/// the chain's volume), the chains summed, with the macros applied through the mappings
/// (macro · depth added to the target's normalized value).
fn render_rack_preset(json: &str, input: &[Vec<f32>; 2]) -> [Vec<f32>; 2] {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    let preset = &v["preset"];
    let macros = preset_params(json);
    let rack = &preset["rack"];
    let chains = rack["chains"].as_array().unwrap();
    let frames = input[0].len();
    if chains.is_empty() {
        // An empty rack passes its input through.
        return input.clone();
    }
    let mut sum = [vec![0.0f32; frames], vec![0.0f32; frames]];
    for (ci, chain) in chains.iter().enumerate() {
        let mut signal = input.clone();
        for (di, dev) in chain["devices"].as_array().unwrap().iter().enumerate() {
            let ty: T = serde_json::from_value(dev["device"]["device"].clone()).unwrap();
            let desc = ether_devices::descriptor(ty);
            let mut params: Vec<(u32, f64)> = dev["params"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.parse().unwrap(), v.as_f64().unwrap()))
                .collect();
            for m in rack["mappings"].as_array().unwrap() {
                let t = &m["target"];
                if m["source"]["type"] != "Macro"
                    || t["chain"].as_u64() != Some(ci as u64)
                    || t["device"].as_u64() != Some(di as u64)
                {
                    continue;
                }
                let id = m["param"].as_u64().unwrap() as u32;
                let index = m["source"]["index"].as_u64().unwrap() as u32;
                let amount = macros
                    .iter()
                    .find(|(i, _)| *i == index)
                    .map_or(0.0, |(_, v)| *v);
                let info = desc.params.iter().find(|q| q.id.0 == id).unwrap();
                let base = params
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map_or(info.default, |(_, v)| *v);
                let norm = (info.to_normalized(base) + amount * m["depth"].as_f64().unwrap())
                    .clamp(0.0, 1.0);
                params.retain(|(i, _)| *i != id);
                params.push((id, info.to_plain(norm)));
            }
            let mut d = make(ty, &params);
            signal = render(&mut *d, &signal, &[], frames).out;
        }
        let gain = 10f32.powf(chain["volume"].as_f64().unwrap_or(0.0) as f32 / 20.0);
        for (acc, x) in sum.iter_mut().zip(&signal) {
            for (a, v) in acc.iter_mut().zip(x) {
                *a += v * gain;
            }
        }
    }
    sum
}

/// The compressors' steady-state reduction on a sine follows their static curve (#845: the
/// branching attack/release smoother sagged between the sine's peaks, 1.0 / 1.7 dB short of
/// 4:1 / 3:1 at 0 dBFS).
#[test]
fn compressors_reach_their_static_curve() {
    let mut bad = Vec::new();
    for ty in [T::Compressor, T::MultibandCompressor] {
        let (worst, detail) = static_curve_error(ty);
        eprintln!("{ty:?}: {worst:.2} dB ({detail})");
        if worst >= 0.3 {
            bad.push(format!("{ty:?}: {worst:.2} dB off ({detail})"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The transient shaper's `Clip` is oversampled (#845: -41 dB of aliasing at +12 dB on a
/// 5 kHz sine), and reports latency only while it is on.
#[test]
fn transient_shaper_clip_aliases_little() {
    let a = shaper_clip_alias_db();
    eprintln!("clip aliasing {a:.1} dB");
    assert!(a < -60.0, "{a:.1} dB");
    assert_eq!(make(T::TransientShaper, &[]).latency(), 0);
    assert!(make(T::TransientShaper, &[(4, 1.0)]).latency() > 0);
}

/// A new note in mono mode doesn't click (#845: the pitch jumped at once, so a sine's slope
/// broke: -50 dB on the synth, poly synth and FM synth; what is left is the envelope's
/// retrigger, as on any retriggered note).
#[test]
fn mono_note_changes_do_not_click() {
    let mut bad = Vec::new();
    for i in instruments().iter().filter(|i| i.mode.is_some()) {
        let click = mono_new_note_click_db(i).unwrap();
        eprintln!("{}: {click:.1} dB", i.name);
        if click >= -60.0 {
            bad.push(format!("{}: {click:.1} dB", i.name));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A cutoff jump on the samplers glides per sample (#845: per render chunk, 8 kHz -> 200 Hz
/// zippered at -47 / -57 dB).
#[test]
fn sampler_cutoff_jumps_do_not_zipper() {
    let mut bad = Vec::new();
    for i in instruments()
        .iter()
        .filter(|i| matches!(i.ty, T::Sampler | T::MultiSampler))
    {
        let z = cutoff_jump_zipper_db(i).unwrap();
        eprintln!("{}: {z:.1} dB", i.name);
        if z >= -62.0 {
            bad.push(format!("{}: {z:.1} dB", i.name));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A reverb impulse response at 100 % wet with `params`: echo density 50 and 100 ms after
/// its onset and the tail's roughness (as in the audit).
fn reverb_ir_quality(params: &[(u32, f64)]) -> (f64, f64, f64) {
    let mut d = make(T::Reverb, params);
    let mut imp = vec![0.0f32; 16];
    imp[0] = 1.0;
    let out = render(&mut *d, &stereo(imp), &[], secs(8.0));
    let l = &out.out[0];
    let onset = l.iter().position(|v| v.abs() > 1e-4).unwrap_or(0) as f64 / f64::from(SR);
    (
        echo_density(l, onset + 0.05),
        echo_density(l, onset + 0.1),
        tail_roughness_db(l, onset + 0.15),
    )
}

/// The algorithmic reverb's early tail is dense at every size, and its tank smooth (#845:
/// at size 100 the tank's first echoes came 60 to 117 ms apart: an echo density of 0.23
/// 50 ms in and 0.59 100 ms in, 0.67 50 ms in at the defaults; undamped, a 0.83 dB rough
/// tail). Damped tails are left out of the roughness bound: they darken as they decay, and
/// the energy of a narrower band fluctuates more over 10 ms (about 1.2 to 1.4 dB here, the
/// same before and after; the bright convolution hall reads 0.6).
#[test]
fn reverb_early_tail_is_dense() {
    let mut bad = Vec::new();
    for (label, params, check_rough) in [
        ("defaults", vec![(5u32, 100.0)], false),
        (
            "size 100, decay 8 s",
            vec![(5, 100.0), (2, 8.0), (1, 100.0)],
            false,
        ),
        ("size 0", vec![(5, 100.0), (1, 0.0)], false),
        (
            "size 100, decay 8 s, damping 0",
            vec![(5, 100.0), (2, 8.0), (1, 100.0), (3, 0.0)],
            true,
        ),
    ] {
        let (d50, d100, rough) = reverb_ir_quality(&params);
        eprintln!("{label}: density {d50:.2} / {d100:.2}, roughness {rough:.2} dB");
        if d50 < 0.8 || d100 < 0.8 || (check_rough && rough > 0.7) {
            bad.push(format!("{label}: {d50:.2} / {d100:.2}, {rough:.2} dB"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
