//! v0.3 (`mpe`): per-note expression reaches a CLAP plugin that takes the CLAP dialect as
//! `clap_event_note_expression` (TUNING in semitones, PRESSURE, BRIGHTNESS). Without MPE on
//! the track, `Pressure` stays poly aftertouch (CONTRACTS.md §13.2); once the track
//! announces its MPE zone in-band, pressure goes out as a note expression too. The test
//! plugin echoes each note expression it receives as MIDI out `[0xA0 | id, key, value·10]`.

use std::path::PathBuf;
use std::sync::OnceLock;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_clap::{ClapPlugin, testing};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::expression::mpe::config_messages;
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::protocol::model::{MpeSettings, NoteExpressionKind};
use ether_core::transport::TransportInfo;

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const ID: &str = "dev.ethereal.test-plugin";
const FRAMES: usize = 64;
/// `CLAP_NOTE_EXPRESSION_*` ids.
const TUNING: u8 = 2;
const BRIGHTNESS: u8 = 5;
const PRESSURE: u8 = 6;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("clap-mpe-tests");
            testing::make_bundle(&dir, "EtherTestMpe")
        })
        .clone()
}

/// One block through `node`; returns the MIDI the plugin sent back.
fn run(node: &mut dyn PluginNode, events: &[ProcessEvent]) -> Vec<(u32, [u8; 3])> {
    let (in_l, in_r) = (vec![0.0f32; FRAMES], vec![0.0f32; FRAMES]);
    let (mut out_l, mut out_r) = (vec![0.0f32; FRAMES], vec![0.0f32; FRAMES]);
    let mut out_events = EventBuffer::with_capacity(64);
    let inputs: [&[f32]; 2] = [&in_l, &in_r];
    let mut outputs: [&mut [f32]; 2] = [&mut out_l, &mut out_r];
    let transport = TransportInfo::STOPPED;
    let mut ctx = ProcessContext {
        sample_rate: 48_000.0,
        frames: FRAMES,
        transport: &transport,
        events,
        out_events: &mut out_events,
    };
    let mut audio = AudioBuffers {
        inputs: &inputs,
        outputs: &mut outputs,
    };
    assert_no_alloc(|| node.process(&mut ctx, &mut audio));
    out_events
        .as_slice()
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Midi { data } => Some((e.offset, data)),
            _ => None,
        })
        .collect()
}

fn at(offset: u32, kind: EventKind) -> ProcessEvent {
    ProcessEvent { offset, kind }
}

fn expr(expression: NoteExpressionKind, value: f32) -> EventKind {
    EventKind::NoteExpression {
        note_id: 7,
        channel: 0,
        key: 60,
        expression,
        value,
    }
}

#[test]
fn note_expressions_reach_a_clap_dialect_plugin() {
    let mut plugin = ClapPlugin::load(&bundle(), ID).expect("load test plugin");
    let mut node = plugin
        .activate(&PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 256,
            max_events_per_block: 64,
        })
        .expect("activate");
    // Start processing (the first block may allocate inside the plugin).
    run(node.as_mut(), &[]);
    let on = EventKind::NoteOn {
        note_id: 7,
        channel: 0,
        key: 60,
        velocity: 1.0,
    };
    let echo = run(
        node.as_mut(),
        &[
            at(0, on),
            at(1, expr(NoteExpressionKind::Pitch, 12.0)),
            at(2, expr(NoteExpressionKind::Timbre, 0.5)),
            at(3, expr(NoteExpressionKind::Pressure, 1.0)),
        ],
    );
    // Pitch and timbre as note expressions; pressure without MPE is poly aftertouch (not
    // echoed: the plugin only echoes note expressions).
    assert_eq!(
        echo,
        vec![
            (1, [0xA0 | TUNING, 60, 120]),
            (2, [0xA0 | BRIGHTNESS, 60, 5])
        ]
    );
    // The track announces MPE: pressure is a note expression now.
    let m = MpeSettings::default();
    let mut events: Vec<ProcessEvent> = config_messages(Some(&m), &m)
        .into_iter()
        .map(|data| at(0, EventKind::Midi { data }))
        .collect();
    events.push(at(5, expr(NoteExpressionKind::Pressure, 0.5)));
    let echo = run(node.as_mut(), &events);
    assert_eq!(echo, vec![(5, [0xA0 | PRESSURE, 60, 5])]);
    plugin.deactivate(node);
}
