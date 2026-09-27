//! In-process AU hosting against Apple's built-in units (present on every Mac):
//! AUDelay `aufx:dely:appl`, AULowpass `aufx:lpas:appl`, DLSMusicDevice `aumu:dls :appl`.
#![cfg(target_os = "macos")]

use std::path::Path;
use std::time::{Duration, Instant};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_au::{AuFormat, AuPlugin};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode, PluginNotification};
use ether_core::protocol::devices::{DeviceCategory, ParamScale, ParamUnit};
use ether_core::protocol::model::{ParamId, PluginFormat};
use ether_core::transport::TransportInfo;
use ether_plugin_host::PluginFormatHost;

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const DELAY: &str = "aufx:dely:appl";
const LOWPASS: &str = "aufx:lpas:appl";
const DLS: &str = "aumu:dls :appl";

// AUDelay params (kDelayParam_*), AULowpass (kLowPassParam_*).
const WET_DRY: ParamId = ParamId(0);
const DELAY_TIME: ParamId = ParamId(1);
const CUTOFF: ParamId = ParamId(0);

const SR: f32 = 48_000.0;
const MAX: usize = 512;

fn config() -> PrepareConfig {
    PrepareConfig {
        sample_rate: SR,
        max_block_size: MAX,
        max_events_per_block: 64,
    }
}

fn load(id: &str) -> Box<dyn PluginController> {
    AuFormat
        .instantiate(Path::new(id), id)
        .unwrap_or_else(|e| panic!("instantiate {id}: {e}"))
}

/// Renders blocks through a node with pre-allocated buffers, allocation-checked.
struct Harness {
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
    out_events: EventBuffer,
    transport: TransportInfo,
}

impl Harness {
    fn new() -> Self {
        Self {
            input: [vec![0.0; MAX], vec![0.0; MAX]],
            output: [vec![0.0; MAX], vec![0.0; MAX]],
            out_events: EventBuffer::with_capacity(64),
            transport: TransportInfo::STOPPED,
        }
    }

    fn run(&mut self, node: &mut dyn PluginNode, frames: usize, events: &[ProcessEvent]) {
        self.out_events.clear();
        let [il, ir] = &self.input;
        let [ol, or] = &mut self.output;
        let inputs: [&[f32]; 2] = [&il[..frames], &ir[..frames]];
        let mut outputs: [&mut [f32]; 2] = [&mut ol[..frames], &mut or[..frames]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames,
            transport: &self.transport,
            events,
            out_events: &mut self.out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| {
            node.process(&mut ctx, &mut audio);
        });
    }

    fn fill(&mut self, f: impl Fn(usize) -> f32, start: usize) {
        for ch in &mut self.input {
            for (i, s) in ch.iter_mut().enumerate() {
                *s = f(start + i);
            }
        }
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn param(offset: u32, p: ParamId, value: f64) -> ProcessEvent {
    ProcessEvent {
        offset,
        kind: EventKind::Param { param: p, value },
    }
}

#[test]
fn registry_lists_builtins() {
    let targets = AuFormat.discover_registry();
    for id in [DELAY, LOWPASS, DLS] {
        assert!(
            targets.iter().any(|t| t == Path::new(id)),
            "{id} not in registry ({} components)",
            targets.len()
        );
    }
    assert!(targets.iter().all(|t| AuFormat.claims(t)));
}

#[test]
fn scan_describes_builtins() {
    let d = &AuFormat.scan(Path::new(DELAY)).expect("scan")[0];
    assert_eq!(d.format, PluginFormat::Au);
    assert_eq!(d.id, DELAY);
    assert_eq!(d.path, DELAY);
    assert_eq!(d.name, "AUDelay");
    assert_eq!(d.vendor, "Apple");
    assert!(!d.version.is_empty());
    assert_eq!(d.features, vec!["aufx"]);
    assert_eq!(d.category, DeviceCategory::AudioEffect);

    let d = &AuFormat.scan(Path::new(DLS)).expect("scan")[0];
    assert_eq!(d.category, DeviceCategory::Instrument);
    assert_eq!(d.features, vec!["aumu"]);

    assert!(AuFormat.scan(Path::new("aufx:none:zzzz")).is_err());
}

#[test]
fn params_are_mapped_stably() {
    let mut a = load(DELAY);
    let mut b = load(DELAY);
    let pa = a.params();
    let pb = b.params();
    assert_eq!(pa, pb, "same unit → same ids and metadata");
    let time = pa.iter().find(|p| p.id == DELAY_TIME).expect("delay time");
    assert_eq!(time.unit, ParamUnit::Seconds);
    assert!(time.min <= 0.001 && time.max >= 1.0, "{time:?}");
    assert!(time.automatable);
    let wet = pa.iter().find(|p| p.id == WET_DRY).expect("wet/dry");
    assert_eq!((wet.min, wet.max), (0.0, 100.0));
    assert!((wet.default - 50.0).abs() < 1e-3, "{wet:?}");

    let lp = load(LOWPASS).params();
    let cutoff = lp.iter().find(|p| p.id == CUTOFF).expect("cutoff");
    assert_eq!(cutoff.unit, ParamUnit::Hertz);
    assert!(matches!(cutoff.scale, ParamScale::Log | ParamScale::Linear));

    let d = a.descriptor();
    assert_eq!(d.name, "AUDelay");
    assert_eq!((d.audio_inputs, d.audio_outputs), (2, 2));
    assert_eq!(d.params, pa);
}

#[test]
fn delay_alters_signal_with_sample_accurate_params() {
    let mut plugin = load(DELAY);
    let mut node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.channels(), (2, 2));
    let mut h = Harness::new();
    h.fill(|_| 1.0, 0);
    // Delay time 1 s by default: for the first blocks the output is the dry part only.
    h.run(&mut *node, MAX, &[]);
    let dry = h.output[0][MAX - 1];
    // Equal-power crossfade at 50%: dry gain cos(pi/4).
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!((dry - half).abs() < 0.05, "50% wet/dry mix, got {dry}");
    // Fully dry from the middle of the next block on. The event is scheduled at its
    // offset (`AUEventSampleTimeImmediate + 256`); Apple's v2 AUDelay applies scheduled
    // values from the start of the buffer, so only "applied by the end" is checked here.
    h.run(&mut *node, MAX, &[param(256, WET_DRY, 0.0)]);
    let after = h.output[0][MAX - 1];
    assert!((after - 1.0).abs() < 0.05, "after the event: {after}");
    h.run(&mut *node, MAX, &[]);
    assert!((h.output[0][0] - 1.0).abs() < 0.05);
    assert!(!node.is_faulted());
    assert!((node.param(WET_DRY).unwrap()).abs() < 1e-9);
    plugin.deactivate(node);

    // Re-activation works.
    let node = plugin.activate(&config()).expect("re-activate");
    plugin.deactivate(node);
}

#[test]
fn lowpass_filters_high_frequencies() {
    let mut plugin = load(LOWPASS);
    let mut node = plugin.activate(&config()).expect("activate");
    let mut h = Harness::new();
    let tone = |i: usize| (i as f32 * 2.0 * std::f32::consts::PI * 12_000.0 / SR).sin();
    h.fill(tone, 0);
    h.run(&mut *node, MAX, &[param(0, CUTOFF, 200.0)]);
    let mut out = 0.0;
    for b in 1..8 {
        h.fill(tone, b * MAX);
        h.run(&mut *node, MAX, &[]);
        out = rms(&h.output[0]);
    }
    let input = rms(&h.input[0]);
    assert!(
        out < input * 0.05,
        "12 kHz through a 200 Hz lowpass: {out} vs {input}"
    );
    assert_eq!(node.latency(), 0);
    let mut notes = Vec::new();
    plugin.poll(&mut notes);
    assert!(
        !notes
            .iter()
            .any(|n| matches!(n, PluginNotification::LatencyChanged { .. })),
        "{notes:?}"
    );
    plugin.deactivate(node);
}

#[test]
fn instrument_plays_notes() {
    let mut plugin = load(DLS);
    let d = plugin.descriptor();
    assert_eq!(d.category, DeviceCategory::Instrument);
    assert!(d.midi_input);
    let mut node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.channels().0, 0, "no audio input");
    let mut h = Harness::new();
    h.run(&mut *node, MAX, &[]);
    assert!(rms(&h.output[0]) < 1e-6, "silent before any note");
    let on = ProcessEvent {
        offset: 100,
        kind: EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 1.0,
        },
    };
    h.run(&mut *node, MAX, &[on]);
    let mut peak = rms(&h.output[0]);
    for _ in 0..10 {
        h.run(&mut *node, MAX, &[]);
        peak = peak.max(rms(&h.output[0]));
    }
    assert!(peak > 1e-3, "a note produces audio (rms {peak})");
    h.run(
        &mut *node,
        MAX,
        &[ProcessEvent {
            offset: 0,
            kind: EventKind::AllNotesOff,
        }],
    );
    assert!(!node.is_faulted());
    plugin.deactivate(node);
}

#[test]
fn state_round_trips() {
    let mut a = load(DELAY);
    a.set_param_value(DELAY_TIME, 0.25).expect("set");
    a.set_param_value(WET_DRY, 80.0).expect("set");
    assert!((a.param_value(DELAY_TIME).unwrap() - 0.25).abs() < 1e-6);
    let state = a.save_state().expect("save");
    assert!(state.starts_with(b"EAUS"));
    let (table, plist) = ether_au::ids::decode_state(&state).unwrap();
    assert!(table.contains(&(1, 1)));
    assert!(plist.starts_with(b"bplist"));

    let mut b = load(DELAY);
    assert!((b.param_value(DELAY_TIME).unwrap() - 1.0).abs() < 1e-6);
    b.load_state(&state).expect("load");
    assert!((b.param_value(DELAY_TIME).unwrap() - 0.25).abs() < 1e-6);
    assert!((b.param_value(WET_DRY).unwrap() - 80.0).abs() < 1e-4);
    assert_eq!(a.params(), {
        // Defaults are first-seen values: compare ids only.
        let mut p = b.params();
        for (x, y) in p.iter_mut().zip(a.params()) {
            x.default = y.default;
        }
        p
    });
    assert!(b.load_state(b"garbage").is_err());
    b.load_state(&[]).expect("empty state is a no-op");
}

#[test]
fn gui_edits_become_notifications() {
    let mut plugin = AuPlugin::load(DELAY).expect("load");
    plugin.simulate_gui_edit(DELAY_TIME, 0.5);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut notes = Vec::new();
    while Instant::now() < deadline {
        plugin.poll(&mut notes);
        if notes.iter().any(|n| {
            matches!(n, PluginNotification::ParamEdited { param, value }
                if *param == DELAY_TIME && (value - 0.5).abs() < 1e-6)
        }) {
            break;
        }
        ether_au::pump_run_loop(Duration::from_millis(10));
    }
    assert!(
        notes.iter().any(
            |n| matches!(n, PluginNotification::ParamEdited { param, .. } if *param == DELAY_TIME)
        ),
        "{notes:?}"
    );
    // Host-side sets are not echoed back as edits.
    notes.clear();
    plugin.set_param_value(DELAY_TIME, 0.3).unwrap();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        plugin.poll(&mut notes);
        ether_au::pump_run_loop(Duration::from_millis(10));
    }
    assert!(
        !notes
            .iter()
            .any(|n| matches!(n, PluginNotification::ParamEdited { .. })),
        "{notes:?}"
    );
}

#[test]
fn editor_needs_main_thread() {
    let mut plugin = load(DELAY);
    // Test threads are never the process main thread: opening must fail cleanly (or report
    // that the unit has no custom view), never crash.
    assert!(plugin.open_editor().is_err());
    plugin.close_editor();
}

#[test]
fn dropping_without_deactivate_is_safe() {
    // Node first (e.g. a panic while active), then the controller.
    let mut plugin = load(DELAY);
    let mut node = plugin.activate(&config()).expect("activate");
    Harness::new().run(&mut *node, 64, &[]);
    drop(node);
    drop(plugin);
    // Controller first; the node (still holding the unit) goes last.
    let mut plugin = load(DELAY);
    let mut node = plugin.activate(&config()).expect("activate");
    drop(plugin);
    Harness::new().run(&mut *node, 64, &[]);
    drop(node);
}
