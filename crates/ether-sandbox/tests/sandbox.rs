//! Out-of-process hosting tests against `ether-clap`'s `ether_test_plugin` fixture.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_clap::testing;
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::model::ParamId;
use ether_core::transport::TransportInfo;
use ether_sandbox::{SandboxOptions, SandboxedPlugin};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const ID: &str = "dev.ethereal.test-plugin";
const GAIN: ParamId = ParamId(1);
const MODE: ParamId = ParamId(2);
const TEMPO: ParamId = ParamId(3);
/// The fixture's own latency.
const PLUGIN_LATENCY: u32 = 64;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("sandbox-tests");
            testing::make_bundle(&dir, "EtherSandboxTest")
        })
        .clone()
}

fn options(wait_budget: Duration) -> SandboxOptions {
    SandboxOptions {
        helper: PathBuf::from(env!("CARGO_BIN_EXE_ether-sandbox-helper")),
        wait_budget: Some(wait_budget),
        ..SandboxOptions::default()
    }
}

/// Deterministic: the audio thread waits (up to 10 s) for every result.
fn spawn() -> SandboxedPlugin {
    spawn_with(options(Duration::from_secs(10)))
}

fn spawn_with(options: SandboxOptions) -> SandboxedPlugin {
    SandboxedPlugin::spawn(&bundle(), ID, &testing::instance_id(), options)
        .expect("spawn sandboxed plugin")
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

/// A scripted stream: per block (frames, events). Input is a ramp over absolute time.
fn script(sizes: &[usize]) -> Vec<(usize, Vec<ProcessEvent>)> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, &frames)| {
            let events = match i {
                2 => vec![param(frames as u32 / 2, GAIN, 0.5)],
                4 => vec![ProcessEvent {
                    offset: 1,
                    kind: EventKind::NoteOn {
                        note_id: 1,
                        channel: 0,
                        key: 60,
                        velocity: 1.0,
                    },
                }],
                6 => vec![
                    ProcessEvent {
                        offset: 0,
                        kind: EventKind::AllNotesOff,
                    },
                    param(3, GAIN, 1.5),
                ],
                _ => Vec::new(),
            };
            (frames, events)
        })
        .collect()
}

/// Render the script through `node`; returns the concatenated left and right outputs.
fn render(
    node: &mut dyn PluginNode,
    max: usize,
    script: &[(usize, Vec<ProcessEvent>)],
) -> Vec<[f32; 2]> {
    let mut h = Harness::new(max);
    let mut t = 0usize;
    let mut out = Vec::new();
    for (frames, events) in script {
        for i in 0..*frames {
            let x = ((t + i) % 97) as f32 / 97.0 - 0.5;
            h.in_l[i] = x;
            h.in_r[i] = -x;
        }
        h.run(node, *frames, events);
        out.extend((0..*frames).map(|i| [h.out_l[i], h.out_r[i]]));
        t += frames;
    }
    out
}

fn compare_delayed(sizes: &[usize], max: usize) {
    let script = script(sizes);

    let mut inproc = ether_clap::instantiate(&bundle(), ID).expect("load in-process");
    let mut node = inproc.activate(&config(max)).unwrap();
    let expected = render(node.as_mut(), max, &script);
    inproc.deactivate(node);

    let mut sandbox = spawn();
    let mut node = sandbox.activate(&config(max)).unwrap();
    assert_eq!(node.latency(), max as u32 + PLUGIN_LATENCY);
    assert_eq!(node.channels(), (2, 2));
    let got = render(node.as_mut(), max, &script);
    assert!(!node.is_faulted());
    assert_eq!(node.param(GAIN), Some(1.5));
    sandbox.deactivate(node);
    assert_eq!(sandbox.underruns(), 0);

    assert_eq!(got.len(), expected.len());
    for (t, s) in got.iter().enumerate() {
        let want = if t < max { [0.0; 2] } else { expected[t - max] };
        assert_eq!(*s, want, "sample {t}");
    }
    // The stream is not trivially silent.
    assert!(expected.iter().any(|s| s[0] != 0.0));
}

#[test]
fn output_equals_in_process_delayed_by_one_block() {
    compare_delayed(&[64; 12], 64);
}

#[test]
fn variable_sub_blocks_keep_a_constant_one_block_delay() {
    compare_delayed(&[128, 32, 96, 1, 127, 64, 128, 50, 128, 7, 100], 128);
}

#[test]
fn params_state_and_notifications_over_ipc() {
    let mut a = spawn();
    let d = a.descriptor();
    assert_eq!(d.name, "Ethereal Test Plugin");
    assert_eq!(
        (d.audio_inputs, d.audio_outputs, d.midi_input),
        (2, 2, true)
    );
    let params = a.params();
    let reference = ether_clap::instantiate(&bundle(), ID).unwrap().params();
    assert_eq!(params, reference);
    assert!(a.has_editor());

    // Inactive set/get.
    assert_eq!(a.param_value(GAIN), Some(1.0));
    a.set_param_value(GAIN, 0.75).unwrap();
    a.set_param_value(MODE, 2.0).unwrap();
    assert_eq!(a.param_value(GAIN), Some(0.75));
    assert_eq!(a.param_value(ParamId(99)), None);

    // State round-trip into another sandboxed instance.
    let state = a.save_state().unwrap();
    assert_eq!(state.len(), 16);
    let mut b = spawn();
    b.load_state(&state).unwrap();
    assert_eq!(b.param_value(GAIN), Some(0.75));
    assert_eq!(b.param_value(MODE), Some(2.0));
    assert!(matches!(b.load_state(&[1, 2]), Err(PluginError::State(_))));

    // Active: events reach the plugin; readback + plugin-side edits come back.
    let mut node = b.activate(&config(64)).unwrap();
    assert!(matches!(
        b.activate(&config(64)),
        Err(PluginError::Activation(_))
    ));
    assert_eq!(node.param(GAIN), Some(0.75));
    let mut h = Harness::new(64);
    h.transport.bpm = 133.0;
    h.transport.playing = true;
    h.run(node.as_mut(), 64, &[param(0, GAIN, 0.25)]);
    // CC 7 = 127: the fixture sets Gain to 2.0 itself, like a GUI edit.
    let cc = [ProcessEvent {
        offset: 3,
        kind: EventKind::Midi {
            data: [0xB0, 7, 127],
        },
    }];
    h.run(node.as_mut(), 64, &cc);
    h.run(node.as_mut(), 64, &[]); // collects the CC block's result
    assert_eq!(b.param_value(TEMPO), Some(133.0));
    assert_eq!(b.param_value(GAIN), Some(2.0));

    let mut out = Vec::new();
    b.poll(&mut out);
    assert_eq!(
        out,
        vec![
            PluginNotification::GestureBegin { param: GAIN },
            PluginNotification::ParamEdited {
                param: GAIN,
                value: 2.0
            },
            PluginNotification::GestureEnd { param: GAIN },
        ]
    );
    out.clear();
    b.poll(&mut out);
    assert!(out.is_empty(), "{out:?}");
    b.deactivate(node);

    // Re-activation keeps state.
    let node = b.activate(&config(64)).unwrap();
    assert_eq!(node.param(GAIN), Some(2.0));
    b.deactivate(node);
}

#[test]
fn editor_is_forwarded_to_the_helper() {
    let mut p = spawn();
    p.open_editor().expect("open editor");
    // The fixture closes its headless window on its 3rd timer tick (helper-side timers).
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !out.contains(&PluginNotification::EditorClosed) {
        assert!(Instant::now() < deadline, "editor never closed: {out:?}");
        p.poll(&mut out);
        std::thread::sleep(Duration::from_millis(5));
    }
    p.open_editor().expect("open again");
    p.close_editor();
}

#[test]
fn helper_crash_is_isolated() {
    let mut p = spawn();
    let mut node = p.activate(&config(64)).unwrap();
    let mut h = Harness::new(64);
    h.in_l.fill(0.5);
    h.in_r.fill(0.5);
    for _ in 0..3 {
        h.run(node.as_mut(), 64, &[]);
    }
    assert_eq!(h.out_l[0], 0.5);

    p.kill_helper();
    for _ in 0..4 {
        h.out_l.fill(1.0);
        h.run(node.as_mut(), 64, &[]);
        assert!(h.out_l.iter().chain(&h.out_r).all(|s| *s == 0.0));
    }
    assert!(node.is_faulted());

    let mut out = Vec::new();
    p.poll(&mut out);
    assert!(
        matches!(out.as_slice(), [PluginNotification::Crashed { .. }]),
        "{out:?}"
    );
    out.clear();
    p.poll(&mut out);
    assert!(out.is_empty(), "Crashed is reported once: {out:?}");

    assert!(matches!(p.save_state(), Err(PluginError::Crashed(_))));
    assert_eq!(p.param_value(GAIN), None);
    p.deactivate(node);
    // Still alive: a new sandboxed instance works.
    let mut q = spawn();
    assert_eq!(q.param_value(GAIN), Some(1.0));
}

#[test]
fn helper_exit_is_detected_without_kill_from_host() {
    let mut p = spawn();
    let mut node = p.activate(&config(64)).unwrap();
    let pid = p.helper_pid();
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while out.is_empty() {
        assert!(Instant::now() < deadline);
        p.poll(&mut out);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        matches!(out[0], PluginNotification::Crashed { .. }),
        "{out:?}"
    );
    let mut h = Harness::new(64);
    h.in_l.fill(0.5);
    h.run(node.as_mut(), 64, &[]);
    assert!(node.is_faulted());
    assert!(h.out_l.iter().all(|s| *s == 0.0));
    p.deactivate(node);
}

fn signal(pid: u32, sig: &str) {
    let status = std::process::Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn late_helper_gives_silence_and_underruns_then_recovers() {
    let mut p = spawn_with(options(Duration::from_millis(100)));
    let mut node = p.activate(&config(64)).unwrap();
    let mut h = Harness::new(64);
    h.in_l.fill(0.5);
    h.in_r.fill(0.5);
    for _ in 0..3 {
        h.run(node.as_mut(), 64, &[]);
    }
    assert_eq!(h.out_l[0], 0.5);
    assert_eq!(p.underruns(), 0);

    // Freeze the helper (after it finished the block in flight, which the next call still
    // collects normally): every later block misses its deadline.
    std::thread::sleep(Duration::from_millis(20));
    signal(p.helper_pid(), "-STOP");
    h.run(node.as_mut(), 64, &[]);
    assert_eq!(h.out_l[0], 0.5);
    for i in 0..3 {
        h.out_l.fill(1.0);
        h.run(node.as_mut(), 64, &[]);
        assert!(h.out_l.iter().all(|s| *s == 0.0), "block {i}");
        assert_eq!(p.underruns(), i + 1);
    }
    assert!(!node.is_faulted());

    // Resume: output comes back, time-aligned, without further underruns.
    signal(p.helper_pid(), "-CONT");
    std::thread::sleep(Duration::from_millis(50));
    let before = p.underruns();
    for _ in 0..3 {
        h.run(node.as_mut(), 64, &[]);
    }
    assert_eq!(p.underruns(), before);
    assert!(h.out_l.iter().all(|s| *s == 0.5), "{:?}", &h.out_l[..4]);
    p.deactivate(node);
}

#[test]
fn no_alloc_on_host_audio_path() {
    // Zero wait budget: exercises the underrun path too (results usually arrive late).
    for budget in [Duration::from_secs(10), Duration::ZERO] {
        let mut p = spawn_with(options(budget));
        let mut node = p.activate(&config(128)).unwrap();
        let mut h = Harness::new(128);
        h.in_l.fill(0.25);
        let events = [
            param(5, GAIN, 0.5),
            ProcessEvent {
                offset: 9,
                kind: EventKind::NoteOn {
                    note_id: 3,
                    channel: 0,
                    key: 64,
                    velocity: 0.5,
                },
            },
        ];
        // First block starts processing in the helper; nothing special on the host side.
        for i in 0..50 {
            let frames = if i % 3 == 0 { 128 } else { 40 };
            assert_no_alloc(|| {
                node.set_param(MODE, 1.0);
                node.reset();
                h.run(node.as_mut(), frames, &events);
            });
        }
        if budget.is_zero() {
            // Some results were late unless the helper is extremely fast; either way no alloc.
            let _ = p.underruns();
        } else {
            assert_eq!(p.underruns(), 0);
        }
        p.kill_helper();
        assert_no_alloc(|| h.run(node.as_mut(), 128, &events));
        p.deactivate(node);
    }
}

#[test]
fn spawn_errors() {
    let e = SandboxedPlugin::spawn(&bundle(), "nope", "t", options(Duration::ZERO)).unwrap_err();
    assert!(matches!(e, PluginError::NotFound(_)), "{e:?}");
    let e = SandboxedPlugin::spawn(
        &bundle(),
        ID,
        "t",
        SandboxOptions {
            helper: Path::new("/nonexistent/ether-sandbox-helper").into(),
            ..options(Duration::ZERO)
        },
    )
    .unwrap_err();
    assert!(matches!(e, PluginError::Ipc(_)), "{e:?}");
}
