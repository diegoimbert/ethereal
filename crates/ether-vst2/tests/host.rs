//! In-process hosting tests against the VST2 test plugins (`examples/ether_vst2_test_plugin.rs`,
//! built as a plain gain effect and as a shell with a 64-bit gain and a synth): scan (plain
//! and shell), params and their text, processing (gain, f64 path, sample-accurate params and
//! MIDI, MIDI out, transport, no allocation), GUI automation, latency, state, editor-less.

use std::path::PathBuf;
use std::sync::OnceLock;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::{ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, DeviceTypeRef};
use ether_core::protocol::model::{ParamId, PluginFormat};
use ether_core::transport::TransportInfo;
use ether_plugin_host::PluginFormatHost;
use ether_vst2::testing::{self, Fixture, GAIN_ID, SHELL_GAIN_ID, SYNTH_ID};
use ether_vst2::{Precision, Vst2Format, Vst2Plugin, instantiate, scan_library};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const GAIN: ParamId = ParamId(0);
const MODE: ParamId = ParamId(1);
const TEMPO: ParamId = ParamId(2);
const FRAMES: usize = 64;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| testing::temp_dir("vst2-host-tests"))
        .clone()
}

fn gain_path() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| testing::make_plugin(&dir(), "EtherGain", Fixture::Plugin))
        .clone()
}

fn shell_path() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| testing::make_plugin(&dir(), "EtherShell", Fixture::Shell))
        .clone()
}

fn config() -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: 256,
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

fn poll(plugin: &mut dyn PluginController) -> Vec<PluginNotification> {
    let mut out = Vec::new();
    plugin.poll(&mut out);
    out
}

fn gain() -> Vst2Plugin {
    Vst2Plugin::load(&gain_path(), GAIN_ID).expect("load gain")
}

#[test]
fn scan_plain_and_shell_libraries() {
    let plugins = scan_library(&gain_path()).expect("scan");
    assert_eq!(plugins.len(), 1, "{plugins:?}");
    let fx = &plugins[0];
    assert_eq!(fx.format, PluginFormat::Vst2);
    assert_eq!(fx.id, GAIN_ID);
    assert_eq!(fx.name, "Ether VST2 Gain");
    assert_eq!(fx.vendor, "Ethereal");
    assert_eq!(fx.version, "1203");
    assert_eq!(fx.features, ["fx"]);
    assert_eq!(fx.category, DeviceCategory::AudioEffect);
    assert_eq!(fx.path, gain_path().to_string_lossy());

    // The shell lists its sub-plugins (not itself), each with its own id and category.
    let shell = scan_library(&shell_path()).expect("scan shell");
    let ids: Vec<_> = shell
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str(), p.category))
        .collect();
    assert_eq!(
        ids,
        [
            (
                SHELL_GAIN_ID,
                "Ether Shell Gain",
                DeviceCategory::AudioEffect
            ),
            (SYNTH_ID, "Ether Shell Synth", DeviceCategory::Instrument),
        ]
    );
    assert_eq!(shell[1].features, ["instrument", "synth"]);
    assert!(
        shell
            .iter()
            .all(|p| p.path == shell_path().to_string_lossy())
    );

    // Through the format host, and discovery of both libraries.
    assert_eq!(Vst2Format.scan(&gain_path()).unwrap(), plugins);
    let found = Vst2Format.discover(&[dir()]);
    assert!(
        found.contains(&gain_path()) && found.contains(&shell_path()),
        "{found:?}"
    );
}

#[test]
fn effect_descriptor_params_and_text() {
    let mut plugin = gain();
    let d = plugin.descriptor();
    assert_eq!(d.name, "Ether VST2 Gain");
    assert_eq!(d.category, DeviceCategory::AudioEffect);
    assert_eq!((d.audio_inputs, d.audio_outputs), (2, 2));
    assert!(!d.midi_input);
    assert_eq!(d.sidechain_inputs, 0);
    assert_eq!(
        d.device_type,
        DeviceTypeRef::Plugin {
            plugin_id: GAIN_ID.into()
        }
    );
    let params = plugin.params();
    let names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Gain", "Mode", "Tempo"]);
    assert_eq!(
        (params[0].min, params[0].max, params[0].default),
        (0.0, 1.0, 0.5)
    );
    assert!(params.iter().all(|p| p.automatable && p.labels.is_none()));

    assert_eq!(plugin.param_value(GAIN), Some(0.5));
    assert_eq!(plugin.param_text(GAIN), Some(("0.0".into(), "dB".into())));
    plugin.set_param_value(GAIN, 0.25).unwrap();
    assert_eq!(plugin.param_value(GAIN), Some(0.25));
    assert_eq!(plugin.param_text(GAIN).unwrap().0, "-6.0");
    assert!(plugin.param_text(ParamId(99)).is_none());
    assert!(matches!(
        plugin.set_param_value(ParamId(99), 0.0),
        Err(PluginError::State(_))
    ));
    // Host-side sets are not echoed as user edits.
    assert!(poll(&mut plugin).is_empty());
    assert_eq!(plugin.precision(), Some(Precision::Single));
    assert_eq!(plugin.tail_samples(), Some(4800));
}

#[test]
fn process_gain_sample_accurate_params_and_transport() {
    let mut plugin = gain();
    plugin.set_param_value(GAIN, 1.0).unwrap(); // 2x
    let mut node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.latency(), 32);
    assert_eq!(node.channels(), (2, 2));
    let mut h = Harness::new(0.5);
    h.transport.bpm = 150.0;
    h.transport.playing = true;
    // A param change in the middle of the block splits it there.
    let events = [param(32, GAIN, 0.25)];
    let status = assert_no_alloc(|| h.run(node.as_mut(), &events));
    assert_eq!(status, ProcessStatus::Continue);
    assert!(
        h.out_l[..32].iter().all(|&s| s == 1.0),
        "{:?}",
        &h.out_l[..34]
    );
    assert!(
        h.out_l[32..].iter().all(|&s| s == 0.25),
        "{:?}",
        &h.out_l[30..]
    );
    assert_eq!(h.out_r, h.out_l);
    assert_eq!(node.param(GAIN), Some(0.25));
    // The plugin read the tempo through audioMasterGetTime.
    assert!((node.param(TEMPO).unwrap() - 0.15).abs() < 1e-6);
    // Params closer than the split threshold to the block start apply at the start.
    let events = [param(3, GAIN, 0.5)];
    assert_no_alloc(|| h.run(node.as_mut(), &events));
    assert!(h.out_l.iter().all(|&s| s == 0.5));
    // Immediate sets.
    node.set_param(MODE, 1.0);
    assert_eq!(node.param(MODE), Some(1.0));
    plugin.deactivate(node);
    assert!(!plugin.is_active());
}

#[test]
fn shell_gain_processes_in_double_precision() {
    let mut plugin = Vst2Plugin::load(&shell_path(), SHELL_GAIN_ID).expect("load shell gain");
    assert_eq!(plugin.descriptor().name, "Ether Shell Gain");
    assert_eq!(plugin.precision(), Some(Precision::Double));
    plugin.set_param_value(GAIN, 0.75).unwrap(); // 1.5x
    let mut node = plugin.activate(&config()).unwrap();
    assert_eq!(node.latency(), 0);
    let mut h = Harness::new(0.5);
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert!(h.out_l.iter().all(|&s| s == 0.75), "{:?}", h.out_l);
    plugin.deactivate(node);
}

#[test]
fn synth_midi_is_sample_accurate_and_echoed() {
    let mut plugin = instantiate(&shell_path(), SYNTH_ID).expect("load synth");
    let d = plugin.descriptor();
    assert_eq!(d.category, DeviceCategory::Instrument);
    assert_eq!((d.audio_inputs, d.audio_outputs), (0, 2));
    assert!(d.midi_input);
    assert_eq!(d.params[0].name, "Volume");
    let mut node = plugin.activate(&config()).unwrap();
    assert_eq!(node.channels(), (0, 2));
    let mut h = Harness::new(0.0);
    let events = [
        ProcessEvent {
            offset: 10,
            kind: EventKind::NoteOn {
                note_id: 1,
                channel: 0,
                key: 60,
                velocity: 1.0,
            },
        },
        ProcessEvent {
            offset: 40,
            kind: EventKind::NoteOff {
                note_id: 1,
                channel: 0,
                key: 60,
                velocity: 0.0,
            },
        },
    ];
    assert_no_alloc(|| h.run(node.as_mut(), &events));
    assert!(
        h.out_l[..10].iter().all(|&s| s == 0.0),
        "{:?}",
        &h.out_l[..12]
    );
    assert!(
        h.out_l[10..40].iter().all(|&s| s == 1.0),
        "{:?}",
        &h.out_l[8..42]
    );
    assert!(h.out_l[40..].iter().all(|&s| s == 0.0));
    // The plugin's MIDI out (audioMasterProcessEvents) comes back as node output events.
    let out = h.out_events.as_slice().to_vec();
    assert_eq!(
        out,
        [ProcessEvent {
            offset: 10,
            kind: EventKind::NoteOn {
                note_id: u32::MAX,
                channel: 0,
                key: 60,
                velocity: 1.0,
            },
        }]
    );
    // A note held over a reset is released (note-off + all notes off).
    let on = [ProcessEvent {
        offset: 0,
        kind: EventKind::Midi {
            data: [0x90, 64, 127],
        },
    }];
    h.run(node.as_mut(), &on);
    assert!(h.out_l.iter().all(|&s| s == 1.0));
    node.reset();
    assert_no_alloc(|| h.run(node.as_mut(), &[]));
    assert!(h.out_l.iter().all(|&s| s == 0.0));
    plugin.deactivate(node);
}

#[test]
fn gui_automation_latency_and_display_notifications() {
    let mut plugin = gain();
    let mut node = plugin.activate(&config()).unwrap();
    assert_eq!(plugin.latency(), 32);

    // A GUI edit: BeginEdit, Automate, EndEdit from the main thread.
    assert_eq!(plugin.vendor_specific(1, 0, 0.0), 1);
    assert_eq!(
        poll(&mut plugin),
        [
            PluginNotification::GestureBegin { param: GAIN },
            PluginNotification::ParamEdited {
                param: GAIN,
                value: 0.75
            },
            PluginNotification::GestureEnd { param: GAIN },
        ]
    );

    // Latency change through audioMasterIOChanged (channel counts unchanged: no restart).
    plugin.vendor_specific(2, 0, 0.0);
    assert_eq!(
        poll(&mut plugin),
        [PluginNotification::LatencyChanged { samples: 128 }]
    );
    assert_eq!(node.latency(), 128);
    assert_eq!(plugin.latency(), 128);

    // audioMasterUpdateDisplay: state changed, params unchanged.
    plugin.vendor_specific(3, 0, 0.0);
    assert_eq!(poll(&mut plugin), [PluginNotification::StateDirty]);
    assert!(poll(&mut plugin).is_empty());

    let mut h = Harness::new(1.0);
    h.run(node.as_mut(), &[]);
    assert!(h.out_l.iter().all(|&s| s == 1.5));
    plugin.deactivate(node);
}

#[test]
fn chunk_state_round_trips() {
    let mut a = gain();
    a.set_param_value(GAIN, 0.125).unwrap();
    a.set_param_value(MODE, 1.0).unwrap();
    let blob = a.save_state().unwrap();
    assert!(blob.starts_with(b"EthVST2\0"));
    assert_eq!(blob[12], 1, "program chunk kind");

    let mut b = gain();
    assert_eq!(b.param_value(GAIN), Some(0.5));
    b.load_state(&blob).unwrap();
    assert_eq!(b.param_value(GAIN), Some(0.125));
    assert_eq!(b.param_value(MODE), Some(1.0));
    // Restoring is not a user edit.
    assert!(poll(&mut b).is_empty());
    b.load_state(&[]).unwrap();
    assert!(matches!(
        b.load_state(b"garbage"),
        Err(PluginError::State(_))
    ));
}

#[test]
fn param_state_round_trips_without_chunks() {
    let mut a = Vst2Plugin::load(&shell_path(), SHELL_GAIN_ID).unwrap();
    a.set_param_value(GAIN, 0.3).unwrap();
    let blob = a.save_state().unwrap();
    assert_eq!(blob[12], 0, "param values kind");
    let mut b = Vst2Plugin::load(&shell_path(), SHELL_GAIN_ID).unwrap();
    b.load_state(&blob).unwrap();
    assert!((b.param_value(GAIN).unwrap() - 0.3).abs() < 1e-6);
}

#[test]
fn editor_less_operation() {
    let mut plugin = gain();
    assert!(!plugin.has_editor());
    assert!(matches!(plugin.open_editor(), Err(PluginError::NoEditor)));
    plugin.close_editor(); // no-op
    // The synth claims an editor; it can only be hosted where host windows exist.
    let synth = instantiate(&shell_path(), SYNTH_ID).unwrap();
    assert_eq!(synth.has_editor(), cfg!(any(target_os = "macos", windows)));
}

#[test]
fn bad_ids() {
    assert!(matches!(
        Vst2Plugin::load(&gain_path(), "not-an-id"),
        Err(PluginError::NotFound(_))
    ));
    // A shell asked for an id it doesn't have hands back the shell itself.
    assert!(matches!(
        Vst2Plugin::load(&shell_path(), "00000001"),
        Err(PluginError::NotFound(_))
    ));
}

#[test]
fn double_activation_is_an_error() {
    let mut plugin = gain();
    let node = plugin.activate(&config()).unwrap();
    assert!(matches!(
        plugin.activate(&config()),
        Err(PluginError::Activation(_))
    ));
    plugin.deactivate(node);
    let node = plugin.activate(&config()).unwrap();
    plugin.deactivate(node);
}
