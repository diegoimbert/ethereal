//! Drum rack node and sampler slice mode / sample range (roadmap v2, `drum-rack`). Every
//! `process` call runs under `assert_no_alloc`.

use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_core::node::NodeData;
use ether_core::protocol::model::{BuiltinDevice, MediaId, Seconds, SliceSettings};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::{SampleResolver, Sampler, drum_rack, sampler};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const LEN: usize = 4800;

/// Frame `i` of channel 0 is `i / frames` (ramp), so the output tells the read position.
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

struct OneSample(Arc<dyn AudioSource>);

impl SampleResolver for OneSample {
    fn resolve(&self, _media: MediaId) -> Option<Arc<dyn AudioSource>> {
        Some(self.0.clone())
    }
}

fn prepared<D: Device + ?Sized>(d: &mut D) {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
}

fn note_on(id: u32, key: u8) -> EventKind {
    EventKind::NoteOn {
        note_id: id,
        channel: 0,
        key,
        velocity: 1.0,
    }
}

/// Render `frames` (stereo) from `input`; `events` are `(absolute frame, kind)`, sorted.
/// `between(block_start, device)` runs before every block (off the no-alloc section).
fn render<D: Device + ?Sized>(
    d: &mut D,
    input: f32,
    events: &[(usize, EventKind)],
    frames: usize,
    mut between: impl FnMut(usize, &mut D),
) -> [Vec<f32>; 2] {
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let inp = vec![input; BLOCK];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut pos = 0;
    while pos < frames {
        between(pos, d);
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
        let inputs: [&[f32]; 2] = [&inp[..n], &inp[..n]];
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

fn slices(markers: &[f64]) -> SliceSettings {
    SliceSettings {
        enabled: true,
        base_note: 36,
        markers: markers.iter().map(|&m| Seconds(m)).collect(),
    }
}

fn sampler_with(s: SliceSettings) -> Sampler {
    let mut d = Sampler::with_slices(Some(Arc::new(Ramp(LEN))), s);
    prepared(&mut d);
    d.set_param(sampler::params::ATTACK, 0.0);
    d
}

fn nonzero_span(x: &[f32]) -> Option<(usize, usize)> {
    let first = x.iter().position(|v| *v != 0.0)?;
    let last = x.iter().rposition(|v| *v != 0.0)?;
    Some((first, last))
}

#[test]
fn slice_mode_plays_one_slice_per_note() {
    // Slices at 0, 960 and 2400 frames (48 kHz).
    let mut s = sampler_with(slices(&[0.0, 0.02, 0.05]));
    // Slice 1 = frames 960..2400, on note 37.
    let out = render(&mut s, 0.0, &[(0, note_on(1, 37))], 4000, |_, _| {});
    let expect = 960.0 / LEN as f32;
    assert!((out[0][0] - expect).abs() < 1e-5, "{}", out[0][0]);
    assert!((out[0][100] - (1060.0 / LEN as f32)).abs() < 1e-5);
    let (_, last) = nonzero_span(&out[0]).unwrap();
    assert_eq!(last, 2400 - 960 - 1, "the slice ends at the next marker");
    // The end is faded (declick): the last sample is far below the ramp value there.
    assert!(out[0][last] < 0.1 * (2399.0 / LEN as f32));

    // The last slice runs to the end of the sample.
    let mut s = sampler_with(slices(&[0.0, 0.02, 0.05]));
    let out = render(&mut s, 0.0, &[(0, note_on(1, 38))], 4000, |_, _| {});
    assert_eq!(nonzero_span(&out[0]).unwrap().1, LEN - 2400 - 1);

    // Keys without a slice are silent.
    for key in [35, 39, 60] {
        let mut s = sampler_with(slices(&[0.0, 0.02, 0.05]));
        let out = render(&mut s, 0.0, &[(0, note_on(1, key))], 2000, |_, _| {});
        assert!(nonzero_span(&out[0]).is_none(), "key {key}");
    }
}

#[test]
fn slice_mode_ignores_root_key_even_when_pitched() {
    let mut s = sampler_with(slices(&[0.0, 0.05]));
    s.set_param(sampler::params::MODE, 1.0);
    let out = render(&mut s, 0.0, &[(0, note_on(1, 37))], 400, |_, _| {});
    // Rate 1: consecutive frames.
    let step = out[0][11] - out[0][10];
    assert!((step - 1.0 / LEN as f32).abs() < 1e-6);
}

#[test]
fn set_data_swaps_slices_without_cutting_notes() {
    let mut s = sampler_with(slices(&[0.0, 0.05]));
    let mut returned: Option<NodeData> = None;
    let out = render(&mut s, 0.0, &[(0, note_on(1, 36))], 2048, |pos, s| {
        if pos == 1024 {
            // New markers mid-note: the sounding voice keeps going.
            returned = s.set_data(Box::new(slices(&[0.0, 0.01, 0.03])));
        }
    });
    let old = returned
        .expect("the previous settings are returned")
        .downcast::<SliceSettings>()
        .expect("SliceSettings");
    assert_eq!(old.markers, vec![Seconds(0.0), Seconds(0.05)]);
    assert_eq!(s.slices().markers.len(), 3);
    for k in [1023, 1024, 2000] {
        assert!(
            (out[0][k] - k as f32 / LEN as f32).abs() < 1e-5,
            "frame {k}"
        );
    }
    // Other data is rejected unchanged.
    let back = s.set_data(Box::new(42u32)).unwrap();
    assert_eq!(*back.downcast::<u32>().unwrap(), 42);
    // New notes use the new markers: slice 1 = 480..1440 (after the first voice ended).
    let out = render(&mut s, 0.0, &[(1000, note_on(2, 37))], 2048, |_, _| {});
    assert!((out[0][1000] - 480.0 / LEN as f32).abs() < 1e-5);
}

#[test]
fn start_end_bound_the_played_range_outside_slice_mode() {
    let mut s = sampler_with(SliceSettings::default());
    s.set_param(sampler::params::START, 50.0);
    s.set_param(sampler::params::END, 75.0);
    let out = render(&mut s, 0.0, &[(0, note_on(1, 60))], 4000, |_, _| {});
    assert!((out[0][0] - 0.5).abs() < 1e-5);
    assert_eq!(nonzero_span(&out[0]).unwrap().1, 1200 - 1);
    // Start >= End plays nothing.
    let mut s = sampler_with(SliceSettings::default());
    s.set_param(sampler::params::START, 60.0);
    s.set_param(sampler::params::END, 60.0);
    let out = render(&mut s, 0.0, &[(0, note_on(1, 60))], 1000, |_, _| {});
    assert!(nonzero_span(&out[0]).is_none());
}

#[test]
fn create_passes_the_slices_to_the_sampler() {
    let media: MediaId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let mut s = ether_devices::create(
        &BuiltinDevice::Sampler {
            sample: Some(media),
            slices: slices(&[0.0, 0.02]),
        },
        &OneSample(Arc::new(Ramp(LEN))),
    );
    prepared(&mut *s);
    s.set_param(sampler::params::ATTACK, 0.0);
    let out = render(&mut *s, 0.0, &[(0, note_on(1, 37))], 256, |_, _| {});
    assert!((out[0][0] - 960.0 / LEN as f32).abs() < 1e-5);
}

#[test]
fn rack_node_applies_volume_and_pan_to_the_pad_mix() {
    let mut r = drum_rack::create();
    prepared(&mut *r);
    assert_eq!(r.descriptor().params.len(), 2);
    let out = render(&mut *r, 0.5, &[], 512, |_, _| {});
    assert_eq!((out[0][10], out[1][10]), (0.5, 0.5), "unity by default");

    r.set_param(drum_rack::params::VOLUME, -6.0);
    r.set_param(drum_rack::params::PAN, 0.5);
    let out = render(&mut *r, 1.0, &[], 512, |_, _| {});
    let g = 10f32.powf(-6.0 / 20.0);
    assert!((out[0][10] - g * 0.5).abs() < 1e-5);
    assert!((out[1][10] - g).abs() < 1e-5);

    // Automation is smoothed: no jump at the event.
    let ev = [(
        100,
        EventKind::Param {
            param: drum_rack::params::VOLUME,
            value: 6.0,
        },
    )];
    r.set_param(drum_rack::params::PAN, 0.0);
    let out = render(&mut *r, 1.0, &ev, 4000, |_, _| {});
    assert!((out[0][101] - out[0][99]).abs() < 0.01);
    assert!((out[0][3999] - 10f32.powf(0.3)).abs() < 1e-3);
}
