//! Sidechain inputs of the built-in Compressor and Limiter (sidechain node): the detector
//! follows the sidechain signal when connected, the main input otherwise. Every `process*`
//! call runs under `assert_no_alloc` (debug builds abort on allocation on the audio path).

use assert_no_alloc::assert_no_alloc;
use ether_core::{
    AudioBuffers, Device, EventBuffer, Node, PrepareConfig, ProcessContext, TransportInfo,
};
use ether_devices::{Compressor, Limiter, compressor, limiter};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn prepared<D: Device>(mut d: D) -> D {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// Render `main` (stereo) through `d`, with `sidechain` (stereo) if given. Returns the left
/// output and the device's gain reduction (dB) after every block.
fn render<D: Device>(
    d: &mut D,
    main: &[f32],
    sidechain: Option<&[f32]>,
    gr: impl Fn(&D) -> f32,
) -> (Vec<f32>, Vec<f32>) {
    let frames = main.len();
    let mut out_l = vec![0.0f32; frames];
    let mut out_r = vec![0.0f32; frames];
    let mut grs = Vec::with_capacity(frames / BLOCK + 1);
    let mut out_events = EventBuffer::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        let inputs: [&[f32]; 2] = [&main[pos..pos + n], &main[pos..pos + n]];
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
        assert_no_alloc(|| match sidechain {
            Some(sc) => {
                let sc: [&[f32]; 2] = [&sc[pos..pos + n], &sc[pos..pos + n]];
                d.process_sidechain(&mut ctx, &mut audio, &sc);
            }
            None => {
                d.process(&mut ctx, &mut audio);
            }
        });
        grs.push(gr(d));
        pos += n;
    }
    (out_l, grs)
}

fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// A "kick": 1 kHz bursts of `len` frames every `period` frames.
fn kick(frames: usize, period: usize, len: usize) -> Vec<f32> {
    let tone = sine(1000.0, 1.0, frames);
    (0..frames)
        .map(|i| if i % period < len { tone[i] } else { 0.0 })
        .collect()
}

fn ducking_compressor() -> Compressor {
    let mut c = prepared(Compressor::new());
    c.set_param(compressor::params::THRESHOLD, -30.0);
    c.set_param(compressor::params::RATIO, 10.0);
    c.set_param(compressor::params::ATTACK, 1.0);
    c.set_param(compressor::params::RELEASE, 50.0);
    c
}

#[test]
fn descriptors_report_a_stereo_sidechain_input() {
    assert_eq!(compressor::descriptor().sidechain_inputs, 2);
    assert_eq!(limiter::descriptor().sidechain_inputs, 2);
    assert_eq!(Compressor::new().sidechain_inputs(), 2);
    assert_eq!(Limiter::new().sidechain_inputs(), 2);
    // The HPF param is appended (ids stay stable).
    let p = compressor::param_infos();
    assert_eq!(p.last().unwrap().id, compressor::params::SIDECHAIN_HPF);
    assert_eq!(p.len(), 6);
}

#[test]
fn compressor_ducks_on_the_sidechain_envelope() {
    // A quiet pad (-34 dBFS, under the -30 dB threshold's knee) is untouched by itself; a
    // kick on the sidechain ducks it, and it recovers between kicks.
    let frames = SR as usize; // 1 s
    let pad = sine(220.0, 0.02, frames); // -34 dBFS: below threshold + knee
    let period = 12_000; // 4 kicks per second
    let len = 2_400; // 50 ms bursts
    let sc = kick(frames, period, len);

    let mut alone = ducking_compressor();
    let (dry, _) = render(&mut alone, &pad, None, Compressor::gain_reduction);
    assert!(
        (peak(&dry) - 0.02).abs() < 1e-4,
        "untouched: {}",
        peak(&dry)
    );

    let mut c = ducking_compressor();
    let (out, gr) = render(&mut c, &pad, Some(&sc), Compressor::gain_reduction);
    for k in 0..frames / period {
        let start = k * period;
        // Near the end of each kick: fully ducked (0 dBFS kick, -30 threshold, 10:1 → ~27 dB).
        let during = peak(&out[start + len - 1_200..start + len]);
        assert!(during < 0.02 * 0.1, "kick {k}: {during}");
        // Just before the next kick (~200 ms later, release 50 ms): recovered.
        let after = peak(&out[start + period - 1_200..start + period]);
        assert!(after > 0.02 * 0.9, "kick {k}: {after}");
        // The gain reduction follows the envelope: high during, ~0 before the next kick.
        let blk_during = (start + len) / BLOCK - 1;
        let blk_after = (start + period) / BLOCK - 1;
        assert!(gr[blk_during] > 20.0, "kick {k}: gr {}", gr[blk_during]);
        assert!(gr[blk_after] < 1.0, "kick {k}: gr {}", gr[blk_after]);
    }

    // A silent sidechain means no reduction even on a loud main signal.
    let loud = sine(220.0, 0.9, frames / 4);
    let silent = vec![0.0; frames / 4];
    let mut c = ducking_compressor();
    let (out, _) = render(&mut c, &loud, Some(&silent), Compressor::gain_reduction);
    assert!((peak(&out) - 0.9).abs() < 1e-3, "{}", peak(&out));
    // Without a sidechain, the main input drives the detector as before.
    let mut c = ducking_compressor();
    let (out, _) = render(&mut c, &loud, None, Compressor::gain_reduction);
    assert!(peak(&out[out.len() / 2..]) < 0.2, "{}", peak(&out));
}

#[test]
fn compressor_sidechain_hpf_ignores_low_end() {
    let frames = SR as usize / 2;
    let pad = sine(220.0, 0.02, frames);
    let sub = sine(40.0, 0.5, frames);
    let gr_with = |hpf: f64| {
        let mut c = ducking_compressor();
        c.set_param(compressor::params::SIDECHAIN_HPF, hpf);
        let (_, gr) = render(&mut c, &pad, Some(&sub), Compressor::gain_reduction);
        gr[gr.len() / 2..].iter().copied().fold(0.0f32, f32::max)
    };
    let off = gr_with(20.0);
    let on = gr_with(500.0);
    assert!(off > 15.0, "{off}");
    assert!(on < off - 12.0, "off {off} dB, on {on} dB");
}

#[test]
fn limiter_detector_follows_the_sidechain() {
    let frames = SR as usize / 2;
    let main = sine(220.0, 0.1, frames); // well under the -0.3 dB ceiling
    let loud = vec![1.0f32; frames]; // +0 dBFS detector: needs ~20 dB of reduction
    let lat = limiter::lookahead_samples(SR) as usize;

    let mut l = prepared(Limiter::new());
    l.set_param(limiter::params::GAIN, 10.0); // main +10 dB → ~0.32, still under ceiling
    let (dry, _) = render(
        &mut l,
        &main,
        Some(&vec![0.0; frames]),
        Limiter::gain_reduction,
    );
    let g = 10f32.powf(10.0 / 20.0);
    assert!((peak(&dry[lat..]) - 0.1 * g).abs() < 1e-3, "{}", peak(&dry));

    let mut l = prepared(Limiter::new());
    l.set_param(limiter::params::GAIN, 10.0);
    let (out, gr) = render(&mut l, &main, Some(&loud), Limiter::gain_reduction);
    // Detector = 1.0 * g (≈ 3.16) vs ceiling ≈ 0.966: gain ≈ 0.305 → main ≈ 0.096.
    let settled = peak(&out[frames / 2..]);
    assert!(settled < 0.1 * g * 0.4, "{settled}");
    assert!(*gr.last().unwrap() > 9.0, "{:?}", gr.last());
    assert!(peak(&out) <= 10f32.powf(-0.3 / 20.0) + 1e-6);
}
