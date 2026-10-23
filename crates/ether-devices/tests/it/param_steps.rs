//! Automation steps (`EventKind::Param { step: true }`, CONTRACTS.md §12.7, issue #302)
//! land whole on their sample in built-in devices: with no smoothing, the output from the
//! step on is the output of a device set to the new value from the start (an output-stage
//! gain has no memory). The same change without `step` (a knob, a grid point) is still
//! smoothed. Every `process` call runs under `assert_no_alloc`.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 512;
const BLOCKS: [usize; 3] = [512, 100, 64];
/// The step's frame (mid-block at every block size).
const AT: usize = 3_037;
const FRAMES: usize = AT + 4_000;

fn device(ty: BuiltinDeviceType, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: MAX_BLOCK,
        max_events_per_block: 64,
    });
    d
}

fn signal(phase: f64) -> Vec<f32> {
    (0..FRAMES)
        .map(|i| {
            let t = std::f64::consts::TAU * 441.0 * i as f64 / f64::from(SR) + phase;
            (0.2 + 0.4 * t.sin()) as f32
        })
        .collect()
}

fn render(d: &mut dyn Device, event: Option<(usize, EventKind)>, block: usize) -> [Vec<f32>; 2] {
    let input = [signal(0.0), signal(1.0)];
    let mut out = [vec![0.0f32; FRAMES], vec![0.0f32; FRAMES]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut events: Vec<ProcessEvent> = Vec::with_capacity(4);
    let mut pos = 0;
    while pos < FRAMES {
        let n = block.min(FRAMES - pos);
        events.clear();
        if let Some((at, kind)) = event
            && at >= pos
            && at < pos + n
        {
            events.push(ProcessEvent {
                offset: (at - pos) as u32,
                kind,
            });
        }
        let transport = TransportInfo {
            sample_time: pos as u64,
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&input[0][pos..pos + n], &input[1][pos..pos + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| d.process(&mut ctx, &mut audio));
        out_events.clear();
        pos += n;
    }
    out
}

/// `id` steps from `from` to `to` at [`AT`]: exact from the step's output sample on, and
/// smoothed without `step`.
fn assert_steps(ty: BuiltinDeviceType, id: ParamId, from: f64, to: f64) {
    for block in BLOCKS {
        let mut old = device(ty, &[(id, from)]);
        let latency = old.latency() as usize;
        let before = render(old.as_mut(), None, block);
        let after = render(device(ty, &[(id, to)]).as_mut(), None, block);
        let event = |step| {
            Some((
                AT,
                EventKind::Param {
                    param: id,
                    value: to,
                    step,
                },
            ))
        };
        let stepped = render(device(ty, &[(id, from)]).as_mut(), event(true), block);
        let smoothed = render(device(ty, &[(id, from)]).as_mut(), event(false), block);
        let due = AT + latency;
        for c in 0..2 {
            for i in 0..due {
                assert!(
                    (stepped[c][i] - before[c][i]).abs() < 1e-6,
                    "{ty:?} (block {block}): sample {i} moved before the step (due {due})"
                );
            }
            for i in due..FRAMES {
                assert!(
                    (stepped[c][i] - after[c][i]).abs() < 1e-5,
                    "{ty:?} (block {block}): sample {i} is {} after the step at {due}, want {}",
                    stepped[c][i],
                    after[c][i]
                );
            }
        }
        assert!(
            (0..2).any(|c| (due..due + 32).any(|i| (smoothed[c][i] - after[c][i]).abs() > 1e-4)),
            "{ty:?} (block {block}): a change without `step` is no longer smoothed"
        );
    }
}

#[test]
fn utility_gain_steps_exactly() {
    use ether_devices::utility::params::GAIN;
    assert_steps(BuiltinDeviceType::Utility, GAIN, 0.0, -18.0);
}

#[test]
fn output_stages_step_exactly() {
    use ether_devices::{eq, fx_color, fx_dynamics, fx_modulation, reverb};
    let cases = [
        (BuiltinDeviceType::Reverb, reverb::params::OUTPUT),
        (BuiltinDeviceType::Eq, eq::params::OUTPUT),
        (BuiltinDeviceType::Chorus, fx_modulation::chorus::OUTPUT),
        (BuiltinDeviceType::Tremolo, fx_modulation::tremolo::OUTPUT),
        (BuiltinDeviceType::Bitcrusher, fx_color::bitcrusher::OUTPUT),
        (BuiltinDeviceType::Saturator, fx_color::saturator::OUTPUT),
        (BuiltinDeviceType::Gate, fx_dynamics::gate::OUTPUT),
        (
            BuiltinDeviceType::TransientShaper,
            fx_dynamics::transient_shaper::OUTPUT,
        ),
    ];
    for (ty, id) in cases {
        assert_steps(ty, id, 0.0, -12.0);
    }
}
