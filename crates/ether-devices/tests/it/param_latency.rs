//! Devices that report latency `L` get their input and their param events at the same
//! timeline time, and their output sits `L` samples late (PDC realigns it). A param change
//! at frame `e` must act on input sample `e`, i.e. change the output from `e + L` on: never
//! before (it would be heard early), and not later (the device's own smoothing may start
//! right there). Each test renders a steady signal twice, with and without one param
//! event, at several block sizes, and checks where the two first differ. Every `process`
//! call runs under `assert_no_alloc`.

use std::f64::consts::TAU;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 512;
/// Block sizes each case runs at (the event lands mid-block in all of them).
const BLOCKS: [usize; 3] = [512, 256, 64];
/// The event's frame.
const AT: usize = 3_037;
/// Slack for the first changed sample: the smoothing's first step can fall on a sample
/// where the signal (nearly) crosses zero.
const SLACK: usize = 4;

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

/// A tone with an offset (nothing in it is ever exactly zero for long).
fn signal(frames: usize, phase: f64) -> Vec<f32> {
    (0..frames)
        .map(|i| (0.2 + 0.4 * (TAU * 997.0 * i as f64 / f64::from(SR) + phase).sin()) as f32)
        .collect()
}

/// Render stereo `input` (and, when given, a stereo sidechain / hardware return) through
/// `d` in blocks of `block`; `events` are `(frame, kind)`, sorted.
fn render(
    d: &mut dyn Device,
    input: &[Vec<f32>; 2],
    sidechain: Option<&[Vec<f32>; 2]>,
    events: &[(usize, EventKind)],
    block: usize,
) -> [Vec<f32>; 2] {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let mut pos = 0;
    while pos < frames {
        let n = block.min(frames - pos);
        block_events.clear();
        for (at, kind) in events {
            if *at >= pos && *at < pos + n {
                block_events.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
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
            events: &block_events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        match sidechain {
            Some(sc) => {
                let sc: [&[f32]; 2] = [&sc[0][pos..pos + n], &sc[1][pos..pos + n]];
                assert_no_alloc(|| d.process_sidechain(&mut ctx, &mut audio, &sc));
            }
            None => {
                assert_no_alloc(|| d.process(&mut ctx, &mut audio));
            }
        }
        out_events.clear();
        pos += n;
    }
    out
}

/// First frame where the renders with and without the events differ.
fn first_change(
    ty: BuiltinDeviceType,
    setup: &[(ParamId, f64)],
    events: &[(usize, EventKind)],
    sidechain: bool,
    block: usize,
) -> (usize, u32) {
    let frames = AT + 6_000;
    let input = [signal(frames, 0.0), signal(frames, 1.0)];
    let ret = [signal(frames, 2.0), signal(frames, 3.0)];
    let sc = sidechain.then_some(&ret);
    let mut a = device(ty, setup);
    let latency = a.latency();
    let reference = render(a.as_mut(), &input, sc, &[], block);
    let mut b = device(ty, setup);
    let changed = render(b.as_mut(), &input, sc, events, block);
    let first = (0..frames)
        .find(|&i| (0..2).any(|c| reference[c][i].to_bits() != changed[c][i].to_bits()))
        .unwrap_or_else(|| panic!("{ty:?}: the event changed nothing"));
    (first, latency)
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param { param: id, value }
}

/// The change reaches the output at `AT + latency + extra` (`extra`: where in the device's
/// path the param acts, past its input), within [`SLACK`].
fn assert_on_time(
    ty: BuiltinDeviceType,
    setup: &[(ParamId, f64)],
    (id, value): (ParamId, f64),
    sidechain: bool,
    extra: isize,
) {
    for block in BLOCKS {
        let (first, latency) = first_change(ty, setup, &[(AT, param(id, value))], sidechain, block);
        assert!(latency > 0, "{ty:?}: no latency in this setup");
        let due = (AT as isize + latency as isize + extra) as usize;
        assert!(
            first >= due,
            "{ty:?} param {id:?} (block {block}): heard {} samples early ({:.2} ms)",
            due - first,
            (due - first) as f32 * 1000.0 / SR,
        );
        assert!(
            first <= due + SLACK,
            "{ty:?} param {id:?} (block {block}): heard {} samples late",
            first - due,
        );
    }
}

#[test]
fn vibrato_output_stage_changes_wait_for_the_latency() {
    use ether_devices::fx_modulation::vibrato as p;
    let ty = BuiltinDeviceType::Vibrato;
    let setup = [(p::MIX, 50.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
    assert_on_time(ty, &setup, (p::MIX, 100.0), false, 0);
    assert_on_time(ty, &setup, (p::DEPTH, 100.0), false, 0);
}

#[test]
fn through_zero_flanger_changes_wait_for_the_latency() {
    use ether_devices::fx_modulation::flanger as p;
    let ty = BuiltinDeviceType::Flanger;
    let setup = [(p::THROUGH_ZERO, 1.0), (p::MIX, 50.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
    assert_on_time(ty, &setup, (p::MIX, 20.0), false, 0);
}

#[test]
fn lookahead_compressor_changes_wait_for_the_latency() {
    use ether_devices::compressor::params as p;
    let ty = BuiltinDeviceType::Compressor;
    let setup = [(p::LOOKAHEAD, 5.0), (p::THRESHOLD, -20.0), (p::MIX, 70.0)];
    assert_on_time(ty, &setup, (p::MAKEUP, 6.0), false, 0);
    assert_on_time(ty, &setup, (p::MIX, 100.0), false, 0);
    assert_on_time(ty, &setup, (p::RANGE, 2.0), false, 0);
    // The detector looks ahead by design; the threshold moves with the audio it acts on.
    assert_on_time(ty, &setup, (p::THRESHOLD, -30.0), false, 0);
}

#[test]
fn lookahead_gate_changes_wait_for_the_latency() {
    use ether_devices::fx_dynamics::gate as p;
    let ty = BuiltinDeviceType::Gate;
    let setup = [(p::LOOKAHEAD, 5.0), (p::THRESHOLD, -40.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
}

#[test]
fn external_effect_and_instrument_changes_wait_for_the_latency() {
    use ether_devices::external::{external_audio_effect as fx, external_instrument as inst};
    let ty = BuiltinDeviceType::ExternalAudioEffect;
    let setup = [(fx::LATENCY, 10.0), (fx::MIX, 50.0)];
    assert_on_time(ty, &setup, (fx::MIX, 20.0), true, 0);
    assert_on_time(ty, &setup, (fx::RETURN_GAIN, -12.0), true, 0);
    assert_on_time(ty, &setup, (fx::INVERT_PHASE, 1.0), true, 0);
    let ty = BuiltinDeviceType::ExternalInstrument;
    assert_on_time(ty, &[(inst::LATENCY, 10.0)], (inst::GAIN, -12.0), true, 0);
}

#[test]
fn saturator_output_stage_changes_wait_for_the_latency() {
    use ether_devices::fx_color::saturator as p;
    let ty = BuiltinDeviceType::Saturator;
    let setup = [(p::MIX, 50.0), (p::DRIVE, 12.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
    assert_on_time(ty, &setup, (p::MIX, 100.0), false, 0);
    assert_on_time(ty, &setup, (p::TONE, 50.0), false, 0);
}

#[test]
fn saturator_drive_acts_where_the_shaper_sees_the_signal() {
    use ether_devices::fx_color::saturator as p;
    let ty = BuiltinDeviceType::Saturator;
    // Oversampling off: the shaper runs on the input, the path delays after it.
    let setup = [(p::MIX, 50.0), (p::DRIVE, 12.0), (p::OVERSAMPLING, 0.0)];
    assert_on_time(ty, &setup, (p::DRIVE, 18.0), false, 0);
    // 4x: the shaper sees the input after the up-sampling filters.
    let setup = [(p::MIX, 50.0), (p::DRIVE, 12.0), (p::OVERSAMPLING, 2.0)];
    assert_in_path(ty, &setup, (p::DRIVE, 18.0));
}

/// A param acting inside a 4x oversampled path: the change spreads through the
/// down-sampling filter's taps, so its first trace may lead `AT + latency` by up to half
/// the round trip (its centre is on time); before the fix it led by the whole latency.
fn assert_in_path(ty: BuiltinDeviceType, setup: &[(ParamId, f64)], (id, value): (ParamId, f64)) {
    for block in BLOCKS {
        let (first, latency) = first_change(ty, setup, &[(AT, param(id, value))], false, block);
        let due = AT + latency as usize;
        assert!(
            first + latency as usize / 2 + 1 >= due,
            "{ty:?} {id:?} (block {block}): {} early",
            due - first
        );
        assert!(
            first <= due,
            "{ty:?} {id:?} (block {block}): {} late",
            first - due
        );
    }
}

#[test]
fn pedal_output_stage_changes_wait_for_the_latency() {
    use ether_devices::fx_color::pedal as p;
    let ty = BuiltinDeviceType::Pedal;
    let setup = [(p::MIX, 50.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
    assert_on_time(ty, &setup, (p::MIX, 100.0), false, 0);
    assert_on_time(ty, &setup, (p::BASS, 80.0), false, 0);
    assert_in_path(ty, &setup, (p::GAIN, 90.0));
}

#[test]
fn auto_filter_output_stage_changes_wait_for_the_latency() {
    use ether_devices::fx_color::auto_filter as p;
    let ty = BuiltinDeviceType::AutoFilter;
    let setup = [(p::MIX, 50.0), (p::LFO_AMOUNT, 0.0), (p::ENV_AMOUNT, 0.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
    assert_on_time(ty, &setup, (p::MIX, 100.0), false, 0);
    assert_in_path(ty, &setup, (p::CUTOFF, 300.0));
}

#[test]
fn erosion_changes_wait_for_the_latency() {
    use ether_devices::fx_color::erosion as p;
    let ty = BuiltinDeviceType::Erosion;
    let setup = [(p::MIX, 50.0), (p::MODE, 0.0)];
    assert_on_time(ty, &setup, (p::OUTPUT, -12.0), false, 0);
    assert_on_time(ty, &setup, (p::AMOUNT, 80.0), false, 0);
}
