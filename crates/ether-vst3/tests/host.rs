//! In-process hosting tests against the `ether_vst3_test_plugin` fixture (see
//! `examples/ether_vst3_test_plugin.rs`): scan, params, processing (gain, notes,
//! sample-accurate params, no allocation), GUI-edit notifications, latency, state.

use std::path::PathBuf;
use std::sync::OnceLock;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::{ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceTypeRef, ParamUnit};
use ether_core::protocol::model::{ParamId, PluginFormat};
use ether_core::transport::TransportInfo;
use ether_plugin_host::PluginFormatHost;
use ether_vst3::testing::{self, EFFECT_ID, INSTRUMENT_ID};
use ether_vst3::{Vst3Format, Vst3Plugin, instantiate, scan_bundle};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const GAIN: ParamId = ParamId(1);
const MODE: ParamId = ParamId(2);
const LATENCY: ParamId = ParamId(3);
const TRIGGER: ParamId = ParamId(4);
const LEVEL: ParamId = ParamId(10);
const INVERT: ParamId = ParamId(11);
const FRAMES: usize = 64;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("vst3-host-tests");
            testing::make_bundle(&dir, "EtherVst3Test")
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

/// Renders one block of constant input through a node with pre-allocated buffers.
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

fn param(offset: u32, p: ParamId, value: f64) -> ProcessEvent {
    ProcessEvent {
        offset,
        kind: EventKind::Param { param: p, value },
    }
}

fn effect() -> Vst3Plugin {
    Vst3Plugin::load(&bundle(), EFFECT_ID).expect("load effect")
}

fn instrument() -> Vst3Plugin {
    Vst3Plugin::load(&bundle(), INSTRUMENT_ID).expect("load instrument")
}

fn poll(plugin: &mut dyn PluginController) -> Vec<PluginNotification> {
    let mut out = Vec::new();
    plugin.poll(&mut out);
    out
}

#[test]
fn scan_lists_audio_module_classes() {
    let plugins = scan_bundle(&bundle()).expect("scan");
    assert_eq!(plugins.len(), 2, "{plugins:?}"); // the controller class is not listed
    let fx = &plugins[0];
    assert_eq!(fx.format, PluginFormat::Vst3);
    assert_eq!(fx.id, EFFECT_ID);
    assert_eq!(fx.name, "Ether VST3 Gain");
    assert_eq!(fx.vendor, "Ethereal");
    assert_eq!(fx.version, "1.2.3");
    assert_eq!(fx.features, ["fx", "dynamics"]);
    assert_eq!(fx.category, DeviceCategory::AudioEffect);
    assert_eq!(fx.path, bundle().to_string_lossy());
    let synth = &plugins[1];
    assert_eq!(synth.id, INSTRUMENT_ID);
    assert_eq!(synth.features, ["instrument", "synth"]);
    assert_eq!(synth.category, DeviceCategory::Instrument);
    // Through the format host too.
    assert_eq!(Vst3Format.scan(&bundle()).unwrap(), plugins);
    assert_eq!(
        Vst3Format.discover(&[bundle().parent().unwrap().to_path_buf()]),
        vec![bundle()]
    );
}

#[test]
fn effect_descriptor_and_params() {
    let mut plugin = effect();
    let params = plugin.params();
    let d = plugin.descriptor();
    assert_eq!(
        d.device_type,
        DeviceTypeRef::Plugin {
            plugin_id: EFFECT_ID.into()
        }
    );
    assert_eq!(d.name, "Ether VST3 Gain");
    assert_eq!(d.category, DeviceCategory::AudioEffect);
    assert_eq!(
        (d.audio_inputs, d.audio_outputs, d.midi_input),
        (2, 2, false)
    );
    assert_eq!(params.len(), 4);

    // Continuous: plain = normalized.
    let gain = &params[0];
    assert_eq!((gain.id, gain.name.as_str()), (GAIN, "Gain"));
    assert_eq!((gain.min, gain.max, gain.default), (0.0, 1.0, 0.5));
    assert!(gain.automatable && !gain.hidden && gain.labels.is_none());

    // Discrete: plain = step index, labels from getParamStringByValue.
    let mode = &params[1];
    assert_eq!((mode.min, mode.max, mode.default), (0.0, 2.0, 0.0));
    assert_eq!(
        mode.labels.as_deref(),
        Some(&["Normal".to_string(), "Invert".into(), "Mute".into()][..])
    );
    let latency = &params[2];
    assert_eq!((latency.max, latency.default), (3.0, 1.0));
    assert_eq!(latency.labels.as_ref().map(Vec::len), Some(4));
    let trigger = &params[3];
    assert!(trigger.hidden && !trigger.automatable);

    assert_eq!(plugin.param_value(GAIN), Some(0.5));
    assert_eq!(plugin.param_value(LATENCY), Some(1.0));
    assert_eq!(plugin.param_value(ParamId(99)), None);
    assert!(poll(&mut plugin).is_empty());
}

#[test]
fn instrument_descriptor_and_params() {
    let mut plugin = instrument();
    let params = plugin.params();
    let d = plugin.descriptor();
    assert_eq!(d.name, "Ether VST3 Synth");
    assert_eq!(d.category, DeviceCategory::Instrument);
    assert_eq!(
        (d.audio_inputs, d.audio_outputs, d.midi_input),
        (0, 2, true)
    );
    assert_eq!(params.len(), 2);
    assert_eq!((params[0].id, params[0].default), (LEVEL, 1.0));
    assert_eq!((params[1].id, params[1].unit), (INVERT, ParamUnit::Toggle));
    assert!(!plugin.has_editor());
    // NoEditor (or, off the main thread on macOS, "must be opened on the main thread").
    assert!(plugin.open_editor().is_err());
}

#[test]
fn unknown_class_and_bundle() {
    assert!(matches!(
        instantiate(&bundle(), "nope"),
        Err(PluginError::NotFound(_))
    ));
    // The controller class is not an audio module.
    assert!(matches!(
        instantiate(&bundle(), "E7E1E4A1000000000000000000000011"),
        Err(PluginError::NotFound(_))
    ));
    assert!(matches!(
        instantiate(&bundle().with_file_name("Missing.vst3"), EFFECT_ID),
        Err(PluginError::NotFound(_))
    ));
}

#[test]
fn process_gain_sample_accurate_params() {
    let mut plugin = effect();
    let mut node = plugin.activate(&config()).expect("activate");
    assert!(matches!(
        plugin.activate(&config()),
        Err(PluginError::Activation(_))
    ));
    assert!(plugin.is_active());
    assert_eq!(node.latency(), 64);
    assert_eq!(node.channels(), (2, 2));
    assert_eq!(node.param(GAIN), Some(0.5));
    assert_eq!(node.param(MODE), Some(0.0));

    let mut h = Harness::new(0.5);
    h.transport.playing = true;
    h.transport.bpm = 133.0;
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert!(h.out_l.iter().all(|s| *s == 0.5), "{:?}", &h.out_l[..4]);

    // Sample-accurate automation (plain values), several params in one block.
    let ev = [
        param(16, MODE, 1.0),
        param(32, GAIN, 0.25),
        param(48, MODE, 0.0),
    ];
    assert_no_alloc(|| h.run(node.as_mut(), &ev));
    assert_eq!(h.out_l[15], 0.5);
    assert_eq!(h.out_l[16], -0.5);
    assert_eq!(h.out_l[31], -0.5);
    assert_eq!(h.out_l[32], -0.25);
    assert_eq!(h.out_l[47], -0.25);
    assert_eq!(h.out_r[48], 0.25);
    assert_eq!(node.param(GAIN), Some(0.25));
    assert_eq!(node.param(MODE), Some(0.0));

    // Immediate set (Device::set_param) arrives with the next block.
    assert_no_alloc(|| node.set_param(MODE, 2.0));
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert!(h.out_l.iter().all(|s| *s == 0.0));

    // The controller follows what the processor received.
    assert!(poll(&mut plugin).is_empty());
    assert_eq!(plugin.param_value(GAIN), Some(0.25));
    assert_eq!(plugin.param_value(MODE), Some(2.0));
    assert!(matches!(
        plugin.set_param_value(GAIN, 1.0),
        Err(PluginError::State(_))
    ));

    assert!(!node.is_faulted());
    plugin.deactivate(node);
    assert!(!plugin.is_active());

    // Re-activation keeps the processor state.
    let mut node = plugin.activate(&config()).expect("re-activate");
    assert_eq!(node.param(GAIN), Some(0.25));
    node.set_param(MODE, 0.0);
    let mut h = Harness::new(1.0);
    h.run(node.as_mut(), &[]);
    assert_eq!(h.out_l[0], 0.5);
    plugin.deactivate(node);
}

#[test]
fn instrument_notes_are_sample_accurate() {
    let mut plugin = instrument();
    let mut node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.channels(), (0, 2));
    assert_eq!(node.latency(), 0);
    let mut h = Harness::new(0.0);
    let on = |offset, key, velocity| ProcessEvent {
        offset,
        kind: EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key,
            velocity,
        },
    };
    assert_no_alloc(|| h.run(node.as_mut(), &[on(10, 60, 0.5)]));
    assert_eq!(h.out_l[9], 0.0);
    assert_eq!(h.out_l[10], 0.5);
    assert_eq!(h.out_r[63], 0.5);

    // AllNotesOff becomes note-offs for the sounding notes.
    let off = [ProcessEvent {
        offset: 20,
        kind: EventKind::AllNotesOff,
    }];
    assert_no_alloc(|| h.run(node.as_mut(), &off));
    assert_eq!(h.out_l[19], 0.5);
    assert_eq!(h.out_l[20], 0.0);

    // Params + raw MIDI notes.
    let ev = [
        param(0, LEVEL, 0.5),
        param(0, INVERT, 1.0),
        ProcessEvent {
            offset: 4,
            kind: EventKind::Midi {
                data: [0x90, 64, 127],
            },
        },
        ProcessEvent {
            offset: 8,
            kind: EventKind::Midi {
                data: [0x80, 64, 0],
            },
        },
    ];
    assert_no_alloc(|| h.run(node.as_mut(), &ev));
    assert_eq!(h.out_l[3], 0.0);
    assert_eq!(h.out_l[4], -0.5);
    assert_eq!(h.out_l[8], 0.0);
    assert_eq!(node.param(INVERT), Some(1.0));
    // Single component: the controller is the processor.
    assert_eq!(plugin.param_value(LEVEL), Some(0.5));

    // reset() releases sounding notes on the next block.
    assert_no_alloc(|| h.run(node.as_mut(), &[on(0, 60, 1.0)]));
    assert_eq!(h.out_l[0], -0.5);
    node.reset();
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert_eq!(h.out_l[0], 0.0);
    plugin.deactivate(node);
}

#[test]
fn gui_edits_become_notifications_and_reach_the_processor() {
    let mut plugin = effect();
    // Inactive: the fixture's "Trigger" param makes its controller edit Gain like its GUI.
    plugin.set_param_value(TRIGGER, 0.75).unwrap();
    assert_eq!(
        poll(&mut plugin),
        vec![
            PluginNotification::GestureBegin { param: GAIN },
            PluginNotification::ParamEdited {
                param: GAIN,
                value: 0.75
            },
            PluginNotification::GestureEnd { param: GAIN },
            PluginNotification::StateDirty,
        ]
    );
    assert!(poll(&mut plugin).is_empty());

    // The processor gets the edit on activation.
    let mut node = plugin.activate(&config()).unwrap();
    let mut h = Harness::new(1.0);
    h.run(node.as_mut(), &[]);
    assert_eq!(h.out_l[0], 1.5);

    // Active: the value reaches the controller through the node, the edit comes back.
    assert_no_alloc(|| h.run(node.as_mut(), &[param(0, TRIGGER, 0.25)]));
    let out = poll(&mut plugin);
    assert!(
        out.contains(&PluginNotification::ParamEdited {
            param: GAIN,
            value: 0.25
        }),
        "{out:?}"
    );
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert_eq!(h.out_l[0], 0.5);
    assert_eq!(node.param(GAIN), Some(0.25));
    plugin.deactivate(node);
}

#[test]
fn latency_changes_are_reported() {
    let mut plugin = effect();
    // Inactive: IMessage to the processor + restartComponent(kLatencyChanged).
    plugin.set_param_value(LATENCY, 2.0).unwrap();
    assert_eq!(
        poll(&mut plugin),
        vec![PluginNotification::LatencyChanged { samples: 128 }]
    );
    let mut node = plugin.activate(&config()).unwrap();
    assert_eq!(node.latency(), 128);

    // Active: automation → controller → restart request.
    let mut h = Harness::new(1.0);
    assert_no_alloc(|| h.run(node.as_mut(), &[param(5, LATENCY, 3.0)]));
    assert_eq!(
        poll(&mut plugin),
        vec![
            PluginNotification::LatencyChanged { samples: 192 },
            PluginNotification::RestartRequested,
        ]
    );
    assert_eq!(node.latency(), 192);
    plugin.deactivate(node);
    let node = plugin.activate(&config()).unwrap();
    assert_eq!(node.latency(), 192);
    plugin.deactivate(node);
}

#[test]
fn state_round_trip() {
    let mut a = effect();
    a.set_param_value(TRIGGER, 0.4).unwrap(); // controller-only state
    poll(&mut a);
    a.set_param_value(GAIN, 0.25).unwrap();
    a.set_param_value(MODE, 1.0).unwrap();
    let state = a.save_state().unwrap();
    assert!(state.starts_with(b"EthVST3\0"), "{state:?}");

    let mut b = effect();
    assert_eq!(b.param_value(GAIN), Some(0.5));
    b.load_state(&state).unwrap();
    assert_eq!(b.param_value(GAIN), Some(0.25));
    assert_eq!(b.param_value(MODE), Some(1.0));
    assert_eq!(b.param_value(LATENCY), Some(1.0));
    assert_eq!(b.param_value(TRIGGER), Some(0.4));
    assert!(poll(&mut b).is_empty());
    assert!(matches!(b.load_state(&[1, 2]), Err(PluginError::State(_))));
    b.load_state(&[]).unwrap();

    // The processor state was restored too (through the trait object).
    let mut c = instantiate(&bundle(), EFFECT_ID).unwrap();
    c.load_state(&state).unwrap();
    let mut node = c.activate(&config()).unwrap();
    assert_eq!(node.param(GAIN), Some(0.25));
    let mut h = Harness::new(1.0);
    h.run(node.as_mut(), &[]);
    assert_eq!(h.out_l[0], -0.5);
    // Saving while active works too.
    let again = c.save_state().unwrap();
    assert_eq!(again, state);
    c.deactivate(node);

    // Single component.
    let mut i = instrument();
    i.set_param_value(LEVEL, 0.5).unwrap();
    i.set_param_value(INVERT, 1.0).unwrap();
    let state = i.save_state().unwrap();
    let mut j = instrument();
    j.load_state(&state).unwrap();
    assert_eq!(j.param_value(LEVEL), Some(0.5));
    assert_eq!(j.param_value(INVERT), Some(1.0));
}

#[test]
fn editor_needs_the_main_thread() {
    let mut plugin = effect();
    // Test threads are not the process main thread: the editor is neither probed
    // (`createView` is main-thread only) nor opened there.
    if cfg!(target_os = "macos") {
        assert!(!plugin.has_editor());
        let e = plugin.open_editor().unwrap_err();
        assert!(matches!(e, PluginError::Load(_)), "{e:?}");
    } else {
        assert!(!plugin.has_editor());
        assert!(matches!(plugin.open_editor(), Err(PluginError::NoEditor)));
    }
    plugin.close_editor();
    assert!(poll(&mut plugin).is_empty());
}

#[test]
fn failing_process_faults_the_node() {
    let mut plugin = instrument();
    let mut node = plugin.activate(&config()).unwrap();
    let mut h = Harness::new(0.0);
    let note = |key| {
        [ProcessEvent {
            offset: 0,
            kind: EventKind::NoteOn {
                note_id: 1,
                channel: 0,
                key,
                velocity: 1.0,
            },
        }]
    };
    // The fixture instrument fails `process` on a note with key 127.
    let bad = note(127);
    let status = assert_no_alloc(|| h.run(node.as_mut(), &bad));
    assert_eq!(status, ProcessStatus::Silent);
    assert!(node.is_faulted());
    assert!(matches!(
        poll(&mut plugin).as_slice(),
        [PluginNotification::Crashed { .. }]
    ));
    assert!(poll(&mut plugin).is_empty()); // reported once
    // A faulted node stays silent.
    h.run(node.as_mut(), &note(60));
    assert!(h.out_l.iter().all(|s| *s == 0.0));
    plugin.deactivate(node);
}

#[test]
fn node_keeps_the_module_alive_after_the_controller_is_dropped() {
    let mut plugin = effect();
    let mut node = plugin.activate(&config()).unwrap();
    // The controller goes away first: the node's processor must still point into a loaded
    // library. (The fixture also aborts at module exit if any of its objects leaked.)
    drop(plugin);
    let mut h = Harness::new(1.0);
    h.run(node.as_mut(), &[]);
    assert_eq!(h.out_l[0], 1.0);
    drop(node);
}
