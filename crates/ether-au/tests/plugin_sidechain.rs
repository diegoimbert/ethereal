//! AU sidechain (input bus 1, CONTRACTS §12.14) against real units. No Apple unit has a
//! second input bus, so the sidechain path runs against Surge XT Effects
//! (`aufx:SFXT:VmbA`) when it is installed; Apple's AUDelay checks the "none" case.
//! (The pull block's bus-1 mapping is unit-tested in `mac/node.rs`.)
#![cfg(target_os = "macos")]

use std::path::Path;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_au::{AuComponentId, AuFormat, registry};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::EventBuffer;
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::transport::TransportInfo;
use ether_plugin_host::PluginFormatHost;

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const DELAY: &str = "aufx:dely:appl";
/// Surge XT Effects (third party, optional): stereo main in/out + a stereo sidechain bus.
const SURGE_FX: &str = "aufx:SFXT:VmbA";
const MAX: usize = 256;

fn config() -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: MAX,
        max_events_per_block: 64,
    }
}

fn load(id: &str) -> Box<dyn PluginController> {
    AuFormat
        .instantiate(Path::new(id), id)
        .unwrap_or_else(|e| panic!("instantiate {id}: {e}"))
}

fn installed(id: &str) -> bool {
    let id = AuComponentId::parse(id).expect("id");
    registry().contains(&id)
}

/// Render `blocks` blocks of noise-ish input, with or without a sidechain; allocation
/// checked. Returns the output peak.
fn render(node: &mut dyn PluginNode, blocks: usize, sidechain: bool) -> f32 {
    let transport = TransportInfo::STOPPED;
    let mut out_events = EventBuffer::with_capacity(64);
    let input: Vec<f32> = (0..MAX)
        .map(|i| ((i * 7919) % 97) as f32 / 97.0 - 0.5)
        .collect();
    let sc: Vec<f32> = (0..MAX)
        .map(|i| if i % 64 < 32 { 0.5 } else { -0.5 })
        .collect();
    let (mut l, mut r) = (vec![0.0f32; MAX], vec![0.0f32; MAX]);
    let mut peak = 0.0f32;
    for _ in 0..blocks {
        let inputs: [&[f32]; 2] = [&input, &input];
        let mut outputs: [&mut [f32]; 2] = [&mut l, &mut r];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: MAX,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        let scs: [&[f32]; 2] = [&sc, &sc];
        assert_no_alloc(|| {
            if sidechain {
                node.process_sidechain(&mut ctx, &mut audio, &scs)
            } else {
                node.process(&mut ctx, &mut audio)
            }
        });
        peak = l.iter().chain(&r).fold(peak, |m, x| m.max(x.abs()));
    }
    peak
}

#[test]
fn a_unit_without_a_second_input_bus_has_no_sidechain() {
    let scanned = AuFormat.scan(Path::new(DELAY)).expect("scan");
    assert_eq!(scanned[0].sidechain_inputs, 0);
    let mut p = load(DELAY);
    assert_eq!(p.descriptor().sidechain_inputs, 0);
    let mut node = p.activate(&config()).expect("activate");
    assert_eq!(node.sidechain_inputs(), 0);
    assert!(render(&mut *node, 4, false).is_finite());
    p.deactivate(node);
}

#[test]
fn surge_fx_takes_a_sidechain_when_installed() {
    if !installed(SURGE_FX) {
        eprintln!("skipped: {SURGE_FX} (Surge XT Effects) is not installed");
        return;
    }
    let scanned = AuFormat.scan(Path::new(SURGE_FX)).expect("scan");
    assert_eq!(scanned[0].sidechain_inputs, 2);
    let mut p = load(SURGE_FX);
    assert_eq!(p.descriptor().sidechain_inputs, 2);
    let mut node = p.activate(&config()).expect("activate");
    assert_eq!(node.sidechain_inputs(), 2);
    assert_eq!(node.descriptor().sidechain_inputs, 2);
    // Both paths render (the bus is enabled, silent without a source) without faulting.
    let without = render(&mut *node, 8, false);
    let with = render(&mut *node, 8, true);
    assert!(without.is_finite() && with.is_finite());
    assert!(!node.is_faulted());
    p.deactivate(node);
}
