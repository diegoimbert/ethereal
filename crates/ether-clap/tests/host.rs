//! In-process hosting tests against the `ether_test_plugin` fixture (see
//! `examples/ether_test_plugin.rs`).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_clap::{ClapPlugin, instantiate, testing};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::{ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceTypeRef};
use ether_core::protocol::model::ParamId;
use ether_core::transport::TransportInfo;

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const ID: &str = "dev.ethereal.test-plugin";
const GAIN: ParamId = ParamId(1);
const MODE: ParamId = ParamId(2);
const TEMPO: ParamId = ParamId(3);
const FRAMES: usize = 64;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("clap-host-tests");
            testing::make_bundle(&dir, "EtherTest")
        })
        .clone()
}

fn config() -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: 256,
        max_events_per_block: 64,
    }
}

/// Render one block of constant `input` through `node`; returns (left output, status, out events).
struct Harness {
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    out_events: EventBuffer,
    transport: TransportInfo,
}

impl Harness {
    fn new(input: f32) -> Self {
        Self {
            in_l: vec![input; FRAMES],
            in_r: vec![input; FRAMES],
            out_l: vec![0.0; FRAMES],
            out_r: vec![0.0; FRAMES],
            out_events: EventBuffer::with_capacity(64),
            transport: TransportInfo::STOPPED,
        }
    }

    fn run(&mut self, node: &mut dyn PluginNode, events: &[ProcessEvent]) -> ProcessStatus {
        self.out_events.clear();
        let inputs: [&[f32]; 2] = [&self.in_l, &self.in_r];
        let mut outputs: [&mut [f32]; 2] = [&mut self.out_l, &mut self.out_r];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: FRAMES,
            transport: &self.transport,
            events,
            out_events: &mut self.out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        node.process(&mut ctx, &mut audio)
    }
}

fn load() -> ClapPlugin {
    ClapPlugin::load(&bundle(), ID).expect("load test plugin")
}

#[test]
fn descriptor_and_params() {
    let mut plugin = load();
    let params = plugin.params();
    let d = plugin.descriptor();
    assert_eq!(
        d.device_type,
        DeviceTypeRef::Plugin {
            plugin_id: ID.into()
        }
    );
    assert_eq!(d.name, "Ethereal Test Plugin");
    assert_eq!(d.category, DeviceCategory::AudioEffect);
    assert_eq!(
        (d.audio_inputs, d.audio_outputs, d.midi_input),
        (2, 2, true)
    );
    assert_eq!(params.len(), 3);

    let gain = &params[0];
    assert_eq!((gain.id, gain.name.as_str()), (GAIN, "Gain"));
    assert_eq!((gain.min, gain.max, gain.default), (0.0, 2.0, 1.0));
    assert_eq!(gain.group.as_deref(), Some("Main"));
    assert!(gain.automatable && !gain.hidden && gain.labels.is_none());

    let mode = &params[1];
    assert_eq!(
        mode.labels.as_deref(),
        Some(&["Clean".to_string(), "Warm".into(), "Hot".into()][..])
    );
    let tempo = &params[2];
    assert!(tempo.hidden && !tempo.automatable);

    assert_eq!(plugin.param_value(GAIN), Some(1.0));
}

#[test]
fn floating_editor_timers_and_close() {
    let mut plugin = load();
    assert!(plugin.has_editor());
    plugin.open_editor().expect("open editor");
    plugin.open_editor().expect("re-show open editor");

    // The fixture closes its (headless) window on its 3rd timer tick.
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !out.contains(&PluginNotification::EditorClosed) {
        assert!(Instant::now() < deadline, "editor never closed: {out:?}");
        plugin.poll(&mut out);
        std::thread::sleep(Duration::from_millis(5));
    }

    // Re-open and close from the host side: no EditorClosed notification then.
    plugin.open_editor().expect("open again");
    plugin.close_editor();
    out.clear();
    std::thread::sleep(Duration::from_millis(50));
    plugin.poll(&mut out);
    assert!(out.is_empty(), "{out:?}");
}

#[test]
fn unknown_plugin_and_bundle() {
    assert!(matches!(
        instantiate(&bundle(), "nope"),
        Err(PluginError::NotFound(_))
    ));
    assert!(matches!(
        instantiate(&bundle().join("missing.clap"), ID),
        Err(PluginError::NotFound(_))
    ));
}

#[test]
fn process_params_notes_transport() {
    let mut plugin = load();
    let mut node = plugin.activate(&config()).expect("activate");
    assert!(matches!(
        plugin.activate(&config()),
        Err(PluginError::Activation(_))
    ));
    assert_eq!(node.latency(), 64);
    assert_eq!(node.channels(), (2, 2));
    assert_eq!(node.param(GAIN), Some(1.0));

    let mut h = Harness::new(0.5);
    h.transport.bpm = 133.0;
    h.transport.playing = true;
    // First block starts processing (may allocate inside the plugin's start_processing).
    assert_eq!(h.run(node.as_mut(), &[]), ProcessStatus::Continue);
    assert!(h.out_l.iter().all(|s| *s == 0.5), "{:?}", &h.out_l[..4]);

    // Sample-accurate automation event (plain value).
    let ev = [ProcessEvent {
        offset: 32,
        kind: EventKind::Param {
            param: GAIN,
            value: 0.5,
        },
    }];
    assert_no_alloc(|| h.run(node.as_mut(), &ev));
    assert_eq!(h.out_l[31], 0.5);
    assert_eq!(h.out_l[32], 0.25);
    assert_eq!(h.out_r[63], 0.25);
    assert_eq!(node.param(GAIN), Some(0.5));

    // Notes: DC offset while held; AllNotesOff releases.
    let on = [ProcessEvent {
        offset: 0,
        kind: EventKind::NoteOn {
            note_id: 7,
            channel: 0,
            key: 60,
            velocity: 1.0,
        },
    }];
    assert_no_alloc(|| h.run(node.as_mut(), &on));
    assert_eq!(h.out_l[0], 0.5);
    let off = [ProcessEvent {
        offset: 0,
        kind: EventKind::AllNotesOff,
    }];
    assert_no_alloc(|| h.run(node.as_mut(), &off));
    assert_eq!(h.out_l[0], 0.25);

    // Immediate set (Device::set_param) is delivered on the next block.
    assert_no_alloc(|| node.set_param(GAIN, 2.0));
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert_eq!(h.out_l[0], 1.0);

    // The plugin saw the transport; params are readable on the main thread while active.
    assert_eq!(plugin.param_value(TEMPO), Some(133.0));
    assert_eq!(plugin.param_value(GAIN), Some(2.0));
    assert!(matches!(
        plugin.set_param_value(GAIN, 1.0),
        Err(PluginError::State(_))
    ));

    assert!(!node.is_faulted());
    plugin.deactivate(node);
    assert!(!plugin.is_active());

    // Re-activation keeps plugin state.
    let mut node = plugin.activate(&config()).expect("re-activate");
    assert_eq!(node.param(GAIN), Some(2.0));
    let mut h = Harness::new(0.25);
    h.run(node.as_mut(), &[]);
    assert_eq!(h.out_l[0], 0.5);
    plugin.deactivate(node);
}

#[test]
fn plugin_side_edits_become_notifications() {
    let mut plugin = load();
    let mut node = plugin.activate(&config()).expect("activate");
    let mut h = Harness::new(1.0);
    // CC 7 = 127 makes the fixture set Gain to 2.0 itself, like a GUI edit.
    let cc = [ProcessEvent {
        offset: 3,
        kind: EventKind::Midi {
            data: [0xB0, 7, 127],
        },
    }];
    h.run(node.as_mut(), &[]);
    assert_no_alloc(|| h.run(node.as_mut(), &cc));
    assert_eq!(node.param(GAIN), Some(2.0));

    let mut out = Vec::new();
    plugin.poll(&mut out);
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
    plugin.poll(&mut out);
    assert!(out.is_empty());
    plugin.deactivate(node);
}

#[test]
fn state_roundtrip_and_inactive_set() {
    let mut a = load();
    a.set_param_value(GAIN, 0.75).unwrap();
    a.set_param_value(MODE, 2.0).unwrap();
    assert_eq!(a.param_value(GAIN), Some(0.75));
    let state = a.save_state().unwrap();
    assert_eq!(state.len(), 16);

    let mut b = load();
    assert_eq!(b.param_value(GAIN), Some(1.0));
    b.load_state(&state).unwrap();
    assert_eq!(b.param_value(GAIN), Some(0.75));
    assert_eq!(b.param_value(MODE), Some(2.0));
    assert!(matches!(b.load_state(&[1, 2]), Err(PluginError::State(_))));

    // Through the trait object too.
    let mut c = instantiate(&bundle(), ID).unwrap();
    c.load_state(&state).unwrap();
    let node = c.activate(&config()).unwrap();
    assert_eq!(node.param(GAIN), Some(0.75));
    c.deactivate(node);
}
