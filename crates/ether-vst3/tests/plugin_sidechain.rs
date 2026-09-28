//! Sidechain (`kAux` input bus) hosting against the fixture's gain effect, whose stereo
//! `Sidechain` aux bus is added to its output when active (CONTRACTS §12.14).

use std::path::PathBuf;
use std::sync::OnceLock;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::EventBuffer;
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::transport::TransportInfo;
use ether_vst3::testing::{self, EFFECT_ID, INSTRUMENT_ID};
use ether_vst3::{Vst3Plugin, scan_bundle};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const FRAMES: usize = 64;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("vst3-sidechain-tests");
            testing::make_bundle(&dir, "EtherVst3Sidechain")
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

/// One block of `input` on both main channels (and `sidechain`, if any): (left, right) out.
fn render(node: &mut dyn PluginNode, input: f32, sidechain: Option<&[&[f32]]>) -> (f32, f32) {
    let transport = TransportInfo::STOPPED;
    let mut out_events = EventBuffer::with_capacity(64);
    let inp = vec![input; FRAMES];
    let (mut l, mut r) = (vec![9.0f32; FRAMES], vec![9.0f32; FRAMES]);
    {
        let inputs: [&[f32]; 2] = [&inp, &inp];
        let mut outputs: [&mut [f32]; 2] = [&mut l, &mut r];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: FRAMES,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| match sidechain {
            Some(sc) => node.process_sidechain(&mut ctx, &mut audio, sc),
            None => node.process(&mut ctx, &mut audio),
        });
    }
    assert!(l.iter().all(|&x| x == l[0]) && r.iter().all(|&x| x == r[0]));
    (l[0], r[0])
}

#[test]
fn scan_and_descriptor_report_the_aux_bus() {
    let scanned = scan_bundle(&bundle()).expect("scan");
    let by_id = |id: &str| scanned.iter().find(|d| d.id == id).expect("scanned class");
    assert_eq!(by_id(EFFECT_ID).sidechain_inputs, 2);
    // The instrument has no aux bus.
    assert_eq!(by_id(INSTRUMENT_ID).sidechain_inputs, 0);

    let mut plugin = Vst3Plugin::load(&bundle(), EFFECT_ID).expect("load");
    assert_eq!(plugin.descriptor().sidechain_inputs, 2);
    let d = plugin.descriptor();
    assert_eq!((d.audio_inputs, d.audio_outputs), (2, 2));
    let node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.sidechain_inputs(), 2);
    assert_eq!(node.descriptor().sidechain_inputs, 2);
    assert_eq!(node.channels(), (2, 2));
    plugin.deactivate(node);

    let instrument = Vst3Plugin::load(&bundle(), INSTRUMENT_ID).expect("load");
    assert_eq!(instrument.descriptor().sidechain_inputs, 0);
}

#[test]
fn the_sidechain_reaches_the_plugin_and_is_silent_without_a_source() {
    let mut plugin = Vst3Plugin::load(&bundle(), EFFECT_ID).expect("load");
    let mut node = plugin.activate(&config()).expect("activate");

    // No source: the (active) aux bus gets silence; unity gain.
    assert_eq!(render(&mut *node, 0.5, None), (0.5, 0.5));

    let (l, r) = (vec![0.25f32; FRAMES], vec![-0.125f32; FRAMES]);
    assert_eq!(render(&mut *node, 0.5, Some(&[&l, &r])), (0.75, 0.375));
    // Mono sidechain feeds both aux channels.
    assert_eq!(render(&mut *node, 0.0, Some(&[&l])), (0.25, 0.25));
    assert_eq!(render(&mut *node, 0.5, None), (0.5, 0.5));
    plugin.deactivate(node);
}
