//! Sidechain (aux input port) hosting against the `ether_test_plugin` fixture, whose second,
//! non-main stereo input port is added to its output (CONTRACTS §12.14).

use std::path::PathBuf;
use std::sync::OnceLock;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_clap::{ClapPlugin, scan_bundle, testing};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::EventBuffer;
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::transport::TransportInfo;

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const ID: &str = "dev.ethereal.test-plugin";
const FRAMES: usize = 64;

fn bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = testing::temp_dir("clap-sidechain-tests");
            testing::make_bundle(&dir, "EtherSidechain")
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
fn scan_and_descriptor_report_the_aux_port() {
    let scanned = scan_bundle(&bundle()).expect("scan");
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].sidechain_inputs, 2);

    let mut plugin = ClapPlugin::load(&bundle(), ID).expect("load");
    assert_eq!(plugin.sidechain_inputs(), 2);
    assert_eq!(plugin.descriptor().sidechain_inputs, 2);
    // Main I/O is unchanged by the aux port.
    let d = plugin.descriptor();
    assert_eq!((d.audio_inputs, d.audio_outputs), (2, 2));

    let node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.sidechain_inputs(), 2);
    assert_eq!(node.descriptor().sidechain_inputs, 2);
    assert_eq!(node.channels(), (2, 2));
    plugin.deactivate(node);
}

#[test]
fn the_sidechain_reaches_the_plugin_and_is_silent_without_a_source() {
    let mut plugin = ClapPlugin::load(&bundle(), ID).expect("load");
    let mut node = plugin.activate(&config()).expect("activate");

    // No source: the aux port gets silence (output = input * gain 1).
    assert_eq!(render(&mut *node, 0.5, None), (0.5, 0.5));

    // Stereo sidechain: each channel is added to its side.
    let (l, r) = (vec![0.25f32; FRAMES], vec![-0.125f32; FRAMES]);
    assert_eq!(render(&mut *node, 0.5, Some(&[&l, &r])), (0.75, 0.375));

    // Mono sidechain feeds both aux channels.
    assert_eq!(render(&mut *node, 0.0, Some(&[&l])), (0.25, 0.25));

    // Back to no source: silence again (nothing sticks in the aux buffers).
    assert_eq!(render(&mut *node, 0.5, None), (0.5, 0.5));
    plugin.deactivate(node);
}
