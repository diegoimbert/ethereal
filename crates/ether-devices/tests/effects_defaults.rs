//! Params appended to the built-in effects default to the sound the
//! effects had before them. Each case renders a fixed stereo signal (tones, noise bursts,
//! silence; transport playing at 120 BPM) through a fresh device, with only pre-existing
//! params set, and compares a hash of the output bits with the one recorded before the pass.
//! Print the current hashes with `PRINT_EFFECT_HASHES=1 cargo test -p ether-devices --test
//! effects_defaults -- --nocapture`.
#![cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code, unused_imports)
)]

use ether_core::protocol::model::{BuiltinDevice, ParamId};
use ether_core::{AudioBuffers, EventBuffer, PrepareConfig, ProcessContext, TransportInfo};
use ether_devices::{NoSamples, create};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
const FRAMES: usize = 48_000;

/// Deterministic test signal: a 220 Hz / 330 Hz tone pair with an envelope, white-noise
/// bursts, a loud transient and a silent stretch (exercises detectors, tails and sleep).
fn signal() -> [Vec<f32>; 2] {
    let mut seed = 0x1234_5678u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let mut l = vec![0.0f32; FRAMES];
    let mut r = vec![0.0f32; FRAMES];
    for i in 0..FRAMES {
        let t = i as f32 / SR;
        let env = if i < 12_000 {
            (t * 8.0).min(1.0) * 0.6
        } else if i < 20_000 {
            0.1
        } else if i < 30_000 {
            0.0
        } else {
            0.9
        };
        let n = noise();
        let burst = if (i / 3000) % 3 == 0 && i < 36_000 {
            0.3 * n
        } else {
            0.0
        };
        let click = if i == 31_000 { 1.0 } else { 0.0 };
        l[i] = env * (std::f32::consts::TAU * 220.0 * t).sin() + burst + click;
        r[i] = env * (std::f32::consts::TAU * 330.0 * t).sin() - burst * 0.5 + click;
        if (30_000..40_000).contains(&i) {
            l[i] = 0.0;
            r[i] = 0.0;
        }
    }
    [l, r]
}

fn fnv(hash: &mut u64, v: f32) {
    for b in v.to_bits().to_le_bytes() {
        *hash ^= u64::from(b);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn render(device: &BuiltinDevice, params: &[(u32, f64)]) -> u64 {
    let mut d = create(device, &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    for &(id, v) in params {
        d.set_param(ParamId(id), v);
    }
    let [l, r] = signal();
    let mut out_l = vec![0.0f32; FRAMES];
    let mut out_r = vec![0.0f32; FRAMES];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut pos = 0;
    while pos < FRAMES {
        let n = BLOCK.min(FRAMES - pos);
        let beats = pos as f64 / f64::from(SR) * 2.0;
        let transport = TransportInfo {
            playing: true,
            sample_time: pos as u64,
            position: beats,
            seconds: pos as f64 / f64::from(SR),
            bpm: 120.0,
            beats_per_sample: 2.0 / f64::from(SR),
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&l[pos..pos + n], &r[pos..pos + n]];
        let mut outputs: [&mut [f32]; 2] = [&mut out_l[pos..pos + n], &mut out_r[pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        d.process(&mut ctx, &mut audio);
        pos += n;
    }
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for i in 0..FRAMES {
        fnv(&mut h, out_l[i]);
        fnv(&mut h, out_r[i]);
    }
    h
}

/// (name, device, pre-existing params set, expected hash).
type Case = (&'static str, BuiltinDevice, Vec<(u32, f64)>, u64);

fn cases() -> Vec<Case> {
    use BuiltinDevice as B;
    vec![
        // Re-recorded for decoupled peak ballistics (the static curve's reduction).
        ("compressor", B::Compressor, vec![], 0xd797_c73d_d217_1d94),
        (
            "compressor-rms",
            B::Compressor,
            vec![(6, 1.0), (0, -30.0)],
            0x00c2_52ab_7625_1d7f,
        ),
        (
            "compressor-expand",
            B::Compressor,
            vec![(6, 2.0), (0, -30.0)],
            0x2c6c_1cac_a5e7_d781,
        ),
        ("delay", B::Delay, vec![], 0x3164_cd86_f101_699b),
        (
            "delay-sync-pingpong",
            B::Delay,
            vec![(0, 1.0), (2, 5.0), (5, 1.0), (3, 80.0)],
            0x57ca_a269_3471_a4dd,
        ),
        // Re-recorded for the multichannel diffuser in front of the tank.
        ("reverb", B::Reverb, vec![], 0x6bdb_535a_e42f_6f0a),
        (
            "reverb-big",
            B::Reverb,
            vec![(1, 90.0), (2, 8.0), (3, 10.0), (0, 80.0)],
            0x712b_0d0d_009b_a228,
        ),
        // Re-recorded for true-peak detection (8 more samples of latency).
        (
            "limiter",
            B::Limiter,
            vec![(0, 12.0)],
            0xe4b4_26cf_4ff8_2a92,
        ),
        ("utility", B::Utility, vec![], 0x2e02_079a_6280_01f8),
        (
            "utility-set",
            B::Utility,
            vec![(0, -6.0), (1, 0.4), (2, 160.0), (3, 1.0)],
            0x8f2c_5555_76f2_7e1f,
        ),
        (
            "utility-mono",
            B::Utility,
            vec![(5, 1.0)],
            0x0e78_7192_c58b_f211,
        ),
        (
            "eq",
            B::Eq,
            vec![(13, 6.0), (23, -4.0)],
            0x2f04_ae61_9984_2e88,
        ),
        ("chorus", B::Chorus, vec![], 0x5f33_bef1_cf98_eec6),
        (
            "chorus-ensemble",
            B::Chorus,
            vec![(0, 1.0), (6, 40.0)],
            0x95af_e4fa_f2ac_5a7e,
        ),
        ("phaser", B::Phaser, vec![], 0x6fc2_6884_a687_3173),
        (
            "phaser-sync",
            B::Phaser,
            vec![(2, 1.0), (3, 8.0), (0, 4.0)],
            0xa008_63a8_28a5_86df,
        ),
        ("flanger", B::Flanger, vec![], 0x993a_3e37_19f5_5bce),
        (
            "flanger-tz",
            B::Flanger,
            vec![(9, 1.0), (5, -60.0)],
            0xa446_27ee_1411_6acc,
        ),
        ("tremolo", B::Tremolo, vec![], 0x1f7e_a3a7_37f9_a12b),
        ("gate", B::Gate, vec![(0, -20.0)], 0x7b57_873b_c802_8553),
        // Re-recorded for ADAA on the oversampled curve, 4x by default.
        ("saturator", B::Saturator, vec![], 0x7739_a71b_6f51_4864),
        // Re-recorded for 4x oversampled (29 samples of latency).
        (
            "auto-filter",
            B::AutoFilter,
            vec![(7, 50.0)],
            0x2d63_dc22_e462_5110,
        ),
        ("bitcrusher", B::Bitcrusher, vec![], 0x7b73_faaa_d35b_f139),
        // Re-recorded for decoupled peak ballistics.
        (
            "multiband",
            B::MultibandCompressor,
            vec![],
            0xbb11_4c24_38cd_0be1,
        ),
        (
            "transient",
            B::TransientShaper,
            vec![(0, 50.0)],
            0xaae4_4d71_57ce_fd69,
        ),
    ]
}

// The hashes were recorded on x86_64 Linux (the gate's platform): libm's transcendental
// functions may round differently elsewhere.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn appended_params_keep_the_default_sound() {
    let print = std::env::var_os("PRINT_EFFECT_HASHES").is_some();
    let mut failures = Vec::new();
    for (name, device, params, expected) in cases() {
        let got = render(&device, &params);
        if print {
            println!("(\"{name}\", 0x{got:016x}),");
        }
        if got != expected {
            failures.push(format!("{name}: 0x{got:016x} != 0x{expected:016x}"));
        }
    }
    if !print {
        assert!(
            failures.is_empty(),
            "default renders changed:\n{}",
            failures.join("\n")
        );
    }
}
