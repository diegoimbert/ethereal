//! Sleep on silence (CONTRACTS.md §13.19) against the fixture's tail effect and instrument
//! (`examples/ether_vst3_test_plugin.rs`): a node stops calling `process` once nothing
//! reached the plugin for its tail and its output is silent, wakes on input, events, param
//! changes and transport jumps, and renders exactly what an always-processing instance
//! renders whenever the plugin reports its tail honestly.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::{ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::protocol::model::ParamId;
use ether_core::transport::TransportInfo;
use ether_vst3::Vst3Plugin;
use ether_vst3::testing::{self, INSTRUMENT_ID, TAIL_ID};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const KIND: ParamId = ParamId(20);
const CALLS: ParamId = ParamId(21);
const IN_FLAGS: ParamId = ParamId(22);
const FRAMES: usize = 64;
/// The tail effect's echo delay (`ECHO` in the fixture).
const ECHO: usize = 300;

const ECHO_KIND: f64 = 0.0;
const GENERATOR: f64 = 1.0;
const LIAR: f64 = 2.0;
const FLAGGED: f64 = 3.0;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("vst3-sleep-tests");
            testing::make_bundle(&dir, "EtherVst3Sleep")
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

/// An active tail effect of `kind`, optionally always processing.
fn tail_fx(kind: f64, always: bool) -> (Vst3Plugin, Box<dyn PluginNode>) {
    let mut plugin = Vst3Plugin::load(&bundle(), TAIL_ID).expect("load tail effect");
    plugin.set_param_value(KIND, kind).expect("kind");
    let node = plugin.activate(&config()).expect("activate");
    node.always_process()
        .expect("VST3 nodes can always process")
        .store(always, Ordering::Relaxed);
    (plugin, node)
}

fn calls(plugin: &mut Vst3Plugin) -> u64 {
    (plugin.param_value(CALLS).expect("calls") * 1e6).round() as u64
}

/// Drives a node like the engine: with `engine_skip`, a node that reported `Silent` is not
/// called while its input is silent, no event reaches it and it requests no wake (the
/// engine's silence skipping); without, it is called every block (the sandbox helper, an
/// analysed node) and must skip the plugin itself.
struct Driver {
    engine_skip: bool,
    asleep: bool,
    out_events: EventBuffer,
    transport: TransportInfo,
    out: Vec<f32>,
    silent_blocks: usize,
}

impl Driver {
    fn new(engine_skip: bool) -> Self {
        Self {
            engine_skip,
            asleep: false,
            out_events: EventBuffer::with_capacity(64),
            transport: TransportInfo::STOPPED,
            out: Vec::new(),
            silent_blocks: 0,
        }
    }

    fn block(&mut self, node: &mut dyn PluginNode, input: &[f32], events: &[ProcessEvent]) {
        let mut l = [0.0f32; FRAMES];
        let mut r = [0.0f32; FRAMES];
        let quiet_in = input.iter().all(|&s| s == 0.0);
        let skip = self.engine_skip
            && self.asleep
            && quiet_in
            && events.is_empty()
            && !node.wake_requested();
        if !skip {
            self.out_events.clear();
            let both: [&[f32]; 2] = [input, input];
            let inputs = if input.is_empty() {
                &both[..0]
            } else {
                &both[..]
            };
            let mut outputs: [&mut [f32]; 2] = [&mut l, &mut r];
            let mut ctx = ProcessContext {
                sample_rate: 48_000.0,
                frames: FRAMES,
                transport: &self.transport,
                events,
                out_events: &mut self.out_events,
            };
            let mut audio = AudioBuffers {
                inputs,
                outputs: &mut outputs,
            };
            let status = assert_no_alloc(|| node.process(&mut ctx, &mut audio));
            self.asleep = status == ProcessStatus::Silent;
        }
        if self.asleep {
            self.silent_blocks += 1;
        }
        self.out.extend_from_slice(&l);
        self.out.extend_from_slice(&r);
        if self.transport.playing {
            self.transport.seconds += FRAMES as f64 / 48_000.0;
        }
    }
}

/// Input: a burst of signal, a long silence, another burst, silence.
fn program(block: usize) -> [f32; FRAMES] {
    let mut b = [0.0f32; FRAMES];
    if block < 3 || (40..42).contains(&block) {
        for (i, s) in b.iter_mut().enumerate() {
            *s = ((block * FRAMES + i) as f32 * 0.37).sin() * 0.5;
        }
    }
    b
}

const BLOCKS: usize = 80;

fn render(kind: f64, always: bool, engine_skip: bool) -> (Driver, u64) {
    let (mut plugin, mut node) = tail_fx(kind, always);
    let mut d = Driver::new(engine_skip);
    for b in 0..BLOCKS {
        d.block(node.as_mut(), &program(b), &[]);
    }
    let n = calls(&mut plugin);
    plugin.deactivate(node);
    (d, n)
}

#[test]
fn an_echo_sleeps_after_its_tail_with_identical_output() {
    let (reference, all) = render(ECHO_KIND, true, true);
    assert_eq!(all, BLOCKS as u64);
    assert_eq!(reference.silent_blocks, 0, "always process never sleeps");
    for engine_skip in [true, false] {
        let (d, n) = render(ECHO_KIND, false, engine_skip);
        assert!(
            d.out == reference.out,
            "sleeping changed the output (engine skip {engine_skip})"
        );
        assert!(d.silent_blocks > 50, "{} silent blocks", d.silent_blocks);
        // Each burst ends at block 2 / 41; the tail (300 samples = 4.7 blocks) and the
        // block the output went silent in are processed, nothing more.
        let tail_blocks = ECHO.div_ceil(FRAMES) as u64;
        assert!(
            n <= 2 * (3 + tail_blocks + 1),
            "{n} process calls (engine skip {engine_skip})"
        );
    }
}

#[test]
fn a_generator_with_an_infinite_tail_never_sleeps() {
    let (reference, _) = render(GENERATOR, true, true);
    let (d, n) = render(GENERATOR, false, true);
    assert_eq!(d.silent_blocks, 0);
    assert_eq!(n, BLOCKS as u64);
    assert!(d.out == reference.out);
    assert!(d.out.iter().filter(|&&s| s == 0.5).count() >= 2 * (BLOCKS * FRAMES / 1000));
}

#[test]
fn a_plugin_without_a_tail_sleeps_unless_it_always_processes() {
    // The liar makes clicks out of nothing but reports no tail: it sleeps (and goes
    // quiet) between two of its clicks, which is what "always process" is for.
    let (reference, all) = render(LIAR, true, true);
    assert_eq!(all, BLOCKS as u64);
    assert_eq!(reference.silent_blocks, 0);
    let (d, n) = render(LIAR, false, true);
    assert!(n < BLOCKS as u64 / 2, "{n} calls");
    assert!(d.out != reference.out);
}

#[test]
fn input_silence_flags_reach_the_plugin_and_output_flags_are_honoured() {
    let (mut plugin, mut node) = tail_fx(FLAGGED, true);
    let mut d = Driver::new(true);
    let loud = [0.25f32; FRAMES];
    d.block(node.as_mut(), &loud, &[]);
    assert_eq!(plugin.param_value(IN_FLAGS), Some(0.0));
    assert!(d.out.iter().all(|&s| s == 0.5));
    d.out.clear();
    d.block(node.as_mut(), &[0.0; FRAMES], &[]);
    assert_eq!(
        plugin.param_value(IN_FLAGS),
        Some(1.0),
        "both channels flagged"
    );
    // The plugin wrote 0.5 but flagged its outputs silent: the host outputs silence.
    assert!(d.out.iter().all(|&s| s == 0.0));
    plugin.deactivate(node);
}

#[test]
fn a_sleeping_plugin_wakes_on_events_param_sets_and_the_transport() {
    let (mut plugin, mut node) = tail_fx(ECHO_KIND, false);
    // Engine-skipped and called-anyway drivers both see the wakes.
    let mut d = Driver::new(false);
    assert_eq!(d.silent_blocks, 0);
    let silence = [0.0f32; FRAMES];
    // The first block delivers the params set before activation; then silent in, silent
    // out (the echo's tail of silence counted from there): asleep.
    for _ in 0..8 {
        d.block(node.as_mut(), &silence, &[]);
    }
    assert!(d.asleep);
    let asleep_calls = calls(&mut plugin);
    for _ in 0..4 {
        d.block(node.as_mut(), &silence, &[]);
    }
    assert_eq!(
        calls(&mut plugin),
        asleep_calls,
        "no process call while asleep"
    );

    // A param event.
    let ev = [ProcessEvent {
        offset: 3,
        kind: EventKind::Param {
            param: ParamId(23),
            value: 0.0,
            step: false,
        },
    }];
    d.block(node.as_mut(), &silence, &ev);
    assert_eq!(calls(&mut plugin), asleep_calls + 1);
    // Wakes for one block, then sleeps again (tail counted from the event).
    let mut n = 0;
    while !d.asleep {
        d.block(node.as_mut(), &silence, &[]);
        n += 1;
        assert!(n < 20);
    }
    let asleep_calls = calls(&mut plugin);

    // An immediate set (modulation, preset) requests a wake.
    node.set_param(ParamId(23), 0.0);
    assert!(node.wake_requested());
    d.block(node.as_mut(), &silence, &[]);
    assert_eq!(calls(&mut plugin), asleep_calls + 1);
    while !d.asleep {
        d.block(node.as_mut(), &silence, &[]);
    }
    let asleep_calls = calls(&mut plugin);

    // Transport start, then a locate while playing.
    d.transport.playing = true;
    d.block(node.as_mut(), &silence, &[]);
    assert_eq!(calls(&mut plugin), asleep_calls + 1);
    while !d.asleep {
        d.block(node.as_mut(), &silence, &[]);
    }
    let asleep_calls = calls(&mut plugin);
    for _ in 0..3 {
        d.block(node.as_mut(), &silence, &[]);
    }
    assert_eq!(
        calls(&mut plugin),
        asleep_calls,
        "playing on silence: still asleep"
    );
    d.transport.seconds = 10.0;
    d.block(node.as_mut(), &silence, &[]);
    assert_eq!(calls(&mut plugin), asleep_calls + 1, "a locate wakes it");

    // "Always process" switched on while asleep.
    while !d.asleep {
        d.block(node.as_mut(), &silence, &[]);
    }
    node.always_process()
        .unwrap()
        .store(true, Ordering::Relaxed);
    assert!(node.wake_requested());
    let before = calls(&mut plugin);
    for _ in 0..5 {
        d.block(node.as_mut(), &silence, &[]);
    }
    assert_eq!(calls(&mut plugin), before + 5);
    assert!(!d.asleep);
    plugin.deactivate(node);
}

#[test]
fn an_instrument_sleeps_only_after_its_notes_end() {
    let mut plugin = Vst3Plugin::load(&bundle(), INSTRUMENT_ID).expect("load instrument");
    let mut node = plugin.activate(&config()).expect("activate");
    let mut d = Driver::new(true);
    let none = [0.0f32; 0];
    let note = |on: bool, offset: u32| ProcessEvent {
        offset,
        kind: if on {
            EventKind::NoteOn {
                note_id: 1,
                channel: 0,
                key: 60,
                velocity: 0.5,
            }
        } else {
            EventKind::NoteOff {
                note_id: 1,
                channel: 0,
                key: 60,
                velocity: 0.0,
            }
        },
    };
    d.block(node.as_mut(), &none, &[]);
    assert!(d.asleep, "no notes: asleep");
    d.out.clear();
    d.block(node.as_mut(), &none, &[note(true, 10)]);
    assert_eq!(d.out[9], 0.0);
    assert_eq!(d.out[10], 0.5, "woken in the same block, sample-accurately");
    for _ in 0..5 {
        d.block(node.as_mut(), &none, &[]);
        assert!(!d.asleep, "a held note keeps it awake");
    }
    d.block(node.as_mut(), &none, &[note(false, 20)]);
    assert!(!d.asleep);
    d.block(node.as_mut(), &none, &[]);
    assert!(d.asleep, "released, no tail, silent: asleep");
    plugin.deactivate(node);
}
