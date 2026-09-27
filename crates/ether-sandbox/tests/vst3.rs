//! Sandboxed VST3 hosting (`ether-sandbox-helper --format vst3`) against `ether-vst3`'s
//! `ether_vst3_test_plugin` fixture.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::model::{ParamId, PluginFormat};
use ether_core::transport::TransportInfo;
use ether_sandbox::{SandboxOptions, SandboxedPlugin};
use ether_vst3::testing::{self, EFFECT_ID, INSTRUMENT_ID};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const GAIN: ParamId = ParamId(1);
const MODE: ParamId = ParamId(2);
const TRIGGER: ParamId = ParamId(4);
/// The fixture effect's default latency.
const PLUGIN_LATENCY: u32 = 64;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("vst3-sandbox-tests");
            testing::make_bundle(&dir, "EtherVst3SandboxTest")
        })
        .clone()
}

fn spawn(id: &str) -> SandboxedPlugin {
    let options = SandboxOptions {
        helper: PathBuf::from(env!("CARGO_BIN_EXE_ether-sandbox-helper")),
        format: PluginFormat::Vst3,
        // Deterministic: the audio thread waits (up to 10 s) for every result.
        wait_budget: Some(Duration::from_secs(10)),
        ..SandboxOptions::default()
    };
    SandboxedPlugin::spawn(&bundle(), id, &testing::instance_id(), options)
        .expect("spawn sandboxed VST3")
}

fn config(max_block_size: usize) -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size,
        max_events_per_block: 64,
    }
}

/// Renders blocks through a node with pre-allocated buffers.
struct Harness {
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    out_events: EventBuffer,
    transport: TransportInfo,
}

impl Harness {
    fn new(max: usize) -> Self {
        Self {
            in_l: vec![0.0; max],
            in_r: vec![0.0; max],
            out_l: vec![0.0; max],
            out_r: vec![0.0; max],
            out_events: EventBuffer::with_capacity(64),
            transport: TransportInfo::STOPPED,
        }
    }

    fn run(&mut self, node: &mut dyn PluginNode, frames: usize, events: &[ProcessEvent]) {
        self.out_events.clear();
        let inputs: [&[f32]; 2] = [&self.in_l[..frames], &self.in_r[..frames]];
        let mut outputs: [&mut [f32]; 2] = [&mut self.out_l[..frames], &mut self.out_r[..frames]];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
            transport: &self.transport,
            events,
            out_events: &mut self.out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        node.process(&mut ctx, &mut audio);
    }
}

fn param(offset: u32, p: ParamId, value: f64) -> ProcessEvent {
    ProcessEvent {
        offset,
        kind: EventKind::Param { param: p, value },
    }
}

/// Render `blocks` blocks of a ramp with scripted param events; returns the left output.
fn render(node: &mut dyn PluginNode, max: usize, blocks: usize) -> Vec<f32> {
    let mut h = Harness::new(max);
    let mut out = Vec::new();
    for b in 0..blocks {
        for i in 0..max {
            let x = ((b * max + i) % 97) as f32 / 97.0 - 0.5;
            h.in_l[i] = x;
            h.in_r[i] = -x;
        }
        let events = match b {
            2 => vec![param(max as u32 / 2, GAIN, 0.25)],
            4 => vec![param(3, MODE, 1.0)],
            _ => Vec::new(),
        };
        assert_no_alloc(|| h.run(node, max, &events));
        out.extend_from_slice(&h.out_l[..max]);
    }
    out
}

#[test]
fn sandboxed_vst3_equals_in_process_delayed_by_one_block() {
    let max = 64;
    let mut inproc = ether_vst3::instantiate(&bundle(), EFFECT_ID).expect("in-process");
    let mut node = inproc.activate(&config(max)).unwrap();
    let expected = render(node.as_mut(), max, 8);
    inproc.deactivate(node);

    let mut sandbox = spawn(EFFECT_ID);
    let mut node = sandbox.activate(&config(max)).unwrap();
    assert_eq!(node.latency(), max as u32 + PLUGIN_LATENCY);
    assert_eq!(node.channels(), (2, 2));
    let got = render(node.as_mut(), max, 8);
    assert!(!node.is_faulted());
    assert_eq!(node.param(GAIN), Some(0.25));
    sandbox.deactivate(node);
    assert_eq!(sandbox.underruns(), 0);

    for (t, s) in got.iter().enumerate() {
        let want = if t < max { 0.0 } else { expected[t - max] };
        assert_eq!(*s, want, "sample {t}");
    }
    assert!(expected.iter().any(|s| *s != 0.0));
}

#[test]
fn params_state_and_gui_edits_over_ipc() {
    let mut a = spawn(EFFECT_ID);
    let d = a.descriptor();
    assert_eq!(d.name, "Ether VST3 Gain");
    assert_eq!((d.audio_inputs, d.audio_outputs), (2, 2));
    let reference = ether_vst3::instantiate(&bundle(), EFFECT_ID)
        .unwrap()
        .params();
    assert_eq!(a.params(), reference);

    a.set_param_value(GAIN, 0.75).unwrap();
    a.set_param_value(MODE, 2.0).unwrap();
    assert_eq!(a.param_value(GAIN), Some(0.75));
    let state = a.save_state().unwrap();

    let mut b = spawn(EFFECT_ID);
    b.load_state(&state).unwrap();
    assert_eq!(b.param_value(GAIN), Some(0.75));
    assert_eq!(b.param_value(MODE), Some(2.0));
    assert!(matches!(b.load_state(&[1, 2]), Err(PluginError::State(_))));

    // A GUI-style edit in the helper comes back as notifications.
    b.set_param_value(TRIGGER, 0.5).unwrap();
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !out.contains(&PluginNotification::GestureEnd { param: GAIN }) {
        assert!(Instant::now() < deadline, "no edit notifications: {out:?}");
        b.poll(&mut out);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(out.contains(&PluginNotification::ParamEdited {
        param: GAIN,
        value: 0.5
    }));
    assert_eq!(b.param_value(GAIN), Some(0.5));
}

#[test]
fn sandboxed_vst3_instrument_plays_notes() {
    let mut p = spawn(INSTRUMENT_ID);
    assert!(p.descriptor().midi_input);
    let mut node = p.activate(&config(64)).unwrap();
    let mut h = Harness::new(64);
    let on = [ProcessEvent {
        offset: 8,
        kind: EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 0.5,
        },
    }];
    h.run(node.as_mut(), 64, &on);
    h.run(node.as_mut(), 64, &[]); // collects the note block (one block of latency)
    assert_eq!(h.out_l[7], 0.0);
    assert_eq!(h.out_l[8], 0.5);
    assert!(!node.is_faulted());
    p.deactivate(node);
}
