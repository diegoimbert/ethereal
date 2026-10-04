//! Sandboxed VST2 hosting (`ether-sandbox-helper --format vst2`) against `ether-vst2`'s test
//! plugins.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::{Path, PathBuf};
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
use ether_vst2::testing::{self, Fixture, GAIN_ID, SYNTH_ID};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const GAIN: ParamId = ParamId(0);
const MODE: ParamId = ParamId(1);
/// The fixture gain's latency (`initialDelay`).
const PLUGIN_LATENCY: u32 = 32;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| testing::temp_dir("vst2-sandbox-tests"))
        .clone()
}

fn gain_path() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| testing::make_plugin(&dir(), "EtherVst2SandboxGain", Fixture::Plugin))
        .clone()
}

fn shell_path() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| testing::make_plugin(&dir(), "EtherVst2SandboxShell", Fixture::Shell))
        .clone()
}

fn spawn(path: &Path, id: &str) -> SandboxedPlugin {
    let options = SandboxOptions {
        helper: PathBuf::from(env!("CARGO_BIN_EXE_ether-sandbox-helper")),
        format: PluginFormat::Vst2,
        // Deterministic: the audio thread waits (up to 10 s) for every result.
        wait_budget: Some(Duration::from_secs(10)),
        ..SandboxOptions::default()
    };
    SandboxedPlugin::spawn(path, id, &testing::instance_id(), options)
        .expect("spawn sandboxed VST2")
}

fn config(max_block_size: usize) -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size,
        max_events_per_block: 64,
    }
}

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
fn sandboxed_vst2_equals_in_process_delayed_by_one_block() {
    let max = 64;
    let mut inproc = ether_vst2::instantiate(&gain_path(), GAIN_ID).expect("in-process");
    let mut node = inproc.activate(&config(max)).unwrap();
    let expected = render(node.as_mut(), max, 8);
    inproc.deactivate(node);

    let mut sandbox = spawn(&gain_path(), GAIN_ID);
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
fn params_and_chunk_state_over_ipc() {
    let mut a = spawn(&gain_path(), GAIN_ID);
    let d = a.descriptor();
    assert_eq!(d.name, "Ether VST2 Gain");
    assert_eq!((d.audio_inputs, d.audio_outputs), (2, 2));
    let reference = ether_vst2::instantiate(&gain_path(), GAIN_ID)
        .unwrap()
        .params();
    assert_eq!(a.params(), reference);

    a.set_param_value(GAIN, 0.75).unwrap();
    a.set_param_value(MODE, 1.0).unwrap();
    assert_eq!(a.param_value(GAIN), Some(0.75));
    let state = a.save_state().unwrap();

    let mut b = spawn(&gain_path(), GAIN_ID);
    b.load_state(&state).unwrap();
    assert_eq!(b.param_value(GAIN), Some(0.75));
    assert_eq!(b.param_value(MODE), Some(1.0));
    assert!(matches!(b.load_state(&[1, 2]), Err(PluginError::State(_))));
    // Host-side sets and restores are not echoed as edits.
    let mut out = Vec::new();
    let until = Instant::now() + Duration::from_millis(200);
    while Instant::now() < until {
        b.poll(&mut out);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !out.iter()
            .any(|n| matches!(n, PluginNotification::ParamEdited { .. })),
        "{out:?}"
    );
}

#[test]
fn sandboxed_vst2_shell_synth_plays_notes() {
    let mut p = spawn(&shell_path(), SYNTH_ID);
    assert!(p.descriptor().midi_input);
    let mut node = p.activate(&config(64)).unwrap();
    let mut h = Harness::new(64);
    let on = [ProcessEvent {
        offset: 8,
        kind: EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 1.0,
        },
    }];
    h.run(node.as_mut(), 64, &on);
    h.run(node.as_mut(), 64, &[]); // collects the note block (one block of latency)
    assert_eq!(h.out_l[7], 0.0);
    assert_eq!(h.out_l[8], 1.0);
    assert!(!node.is_faulted());
    p.deactivate(node);
}
