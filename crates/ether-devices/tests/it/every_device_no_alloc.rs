//! Allocation audit of the audio path: every built-in device type renders under
//! `assert_no_alloc` (debug builds abort on an allocation) with noise in, notes, and a
//! change of every parameter (smoothed, then as automation steps), at block sizes that split
//! the device's internal blocks differently. Devices with a sidechain get one.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 512;
const FRAMES: usize = 24_000;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// `(frame, event)` sorted: notes every 3000 frames, then a change of every param.
fn events(d: &dyn Device, step: bool, rng: &mut Rng) -> Vec<(usize, EventKind)> {
    let mut ev: Vec<(usize, EventKind)> = Vec::new();
    for (k, at) in (0..FRAMES).step_by(3_000).enumerate() {
        let key = 48 + (k % 24) as u8;
        ev.push((
            at,
            EventKind::NoteOn {
                note_id: k as u32,
                channel: 0,
                key,
                velocity: 0.8,
            },
        ));
        ev.push((
            at + 1_500,
            EventKind::NoteOff {
                note_id: k as u32,
                channel: 0,
                key,
                velocity: 0.5,
            },
        ));
    }
    for (k, p) in d.descriptor().params.iter().enumerate() {
        let v = p.min + (p.max - p.min) * f64::from(rng.next());
        ev.push((
            (2_000 + k * 263) % (FRAMES - 1),
            EventKind::Param {
                param: p.id,
                value: v,
                step,
            },
        ));
    }
    ev.sort_by_key(|e| e.0);
    ev
}

fn render(d: &mut dyn Device, events: &[(usize, EventKind)], block: usize, rng: &mut Rng) {
    let noise: Vec<f32> = (0..FRAMES).map(|_| rng.next() - 0.5).collect();
    let mut out = [vec![0.0f32; MAX_BLOCK], vec![0.0f32; MAX_BLOCK]];
    let mut out_events = EventBuffer::with_capacity(256);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(256);
    let sidechain = d.sidechain_inputs() > 0;
    let mut pos = 0;
    while pos < FRAMES {
        let n = block.min(FRAMES - pos);
        block_events.clear();
        for (at, kind) in events.iter().filter(|e| e.0 >= pos && e.0 < pos + n) {
            block_events.push(ProcessEvent {
                offset: (*at - pos) as u32,
                kind: *kind,
            });
        }
        let transport = TransportInfo {
            playing: true,
            sample_time: pos as u64,
            bpm: 120.0,
            beats_per_sample: 2.0 / SR as f64,
            position: pos as f64 * 2.0 / SR as f64,
            ..TransportInfo::STOPPED
        };
        let input = &noise[pos..pos + n];
        let inputs: [&[f32]; 2] = [input, input];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][..n], &mut r[0][..n]];
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
            if sidechain {
                d.process_sidechain(&mut ctx, &mut audio, &[input, input]);
            } else {
                d.process(&mut ctx, &mut audio);
            }
        });
        out_events.clear();
        pos += n;
    }
}

#[test]
fn every_builtin_device_renders_without_allocating() {
    let mut rng = Rng(11);
    for ty in BuiltinDeviceType::ALL {
        for (block, step) in [(128, false), (37, true), (MAX_BLOCK, false)] {
            let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
            d.prepare(&PrepareConfig {
                sample_rate: SR,
                max_block_size: MAX_BLOCK,
                max_events_per_block: 256,
            });
            let ev = events(d.as_ref(), step, &mut rng);
            render(d.as_mut(), &ev, block, &mut rng);
        }
    }
}
