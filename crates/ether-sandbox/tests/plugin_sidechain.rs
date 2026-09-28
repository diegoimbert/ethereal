//! Sidechain through the sandbox (CONTRACTS §12.14): the shm region's sidechain channels
//! reach the helper's plugin, for the CLAP and VST3 fixtures (each adds its aux input to its
//! output), and an aux bus without a source stays silent.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::EventBuffer;
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::protocol::model::PluginFormat;
use ether_core::transport::TransportInfo;
use ether_sandbox::{SandboxOptions, SandboxedPlugin};

#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

const CLAP_ID: &str = "dev.ethereal.test-plugin";
const MAX: usize = 128;

fn clap_bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = ether_clap::testing::temp_dir("sandbox-sidechain-clap");
            ether_clap::testing::make_bundle(&dir, "EtherSandboxSidechain")
        })
        .clone()
}

fn vst3_bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let dir = ether_vst3::testing::temp_dir("sandbox-sidechain-vst3");
            ether_vst3::testing::make_bundle(&dir, "EtherSandboxSidechainVst3")
        })
        .clone()
}

fn spawn(bundle: &Path, id: &str, format: PluginFormat) -> SandboxedPlugin {
    let options = SandboxOptions {
        helper: PathBuf::from(env!("CARGO_BIN_EXE_ether-sandbox-helper")),
        format,
        // Deterministic: the audio thread waits for every result.
        wait_budget: Some(Duration::from_secs(10)),
        ..SandboxOptions::default()
    };
    SandboxedPlugin::spawn(bundle, id, &ether_clap::testing::instance_id(), options)
        .expect("spawn sandboxed plugin")
}

fn config() -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: MAX,
        max_events_per_block: 64,
    }
}

/// Render 3 full blocks of `input` (and `sidechain`): the output of the last one, which is
/// the second block's result (the sandbox is one block late). Allocation checked.
fn render(node: &mut dyn PluginNode, input: f32, sidechain: Option<&[&[f32]]>) -> (f32, f32) {
    let transport = TransportInfo::STOPPED;
    let mut out_events = EventBuffer::with_capacity(64);
    let inp = vec![input; MAX];
    let (mut l, mut r) = (vec![9.0f32; MAX], vec![9.0f32; MAX]);
    for _ in 0..3 {
        let inputs: [&[f32]; 2] = [&inp, &inp];
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
        assert_no_alloc(|| match sidechain {
            Some(sc) => node.process_sidechain(&mut ctx, &mut audio, sc),
            None => node.process(&mut ctx, &mut audio),
        });
    }
    assert!(l.iter().all(|&x| x == l[0]), "{l:?}");
    assert!(r.iter().all(|&x| x == r[0]), "{r:?}");
    (l[0], r[0])
}

fn check(mut plugin: SandboxedPlugin) {
    assert_eq!(plugin.descriptor().sidechain_inputs, 2);
    let mut node = plugin.activate(&config()).expect("activate");
    assert_eq!(node.sidechain_inputs(), 2);
    assert_eq!(node.descriptor().sidechain_inputs, 2);

    // No source: silent aux bus (unity gain).
    assert_eq!(render(&mut *node, 0.5, None), (0.5, 0.5));
    let (l, r) = (vec![0.25f32; MAX], vec![-0.125f32; MAX]);
    assert_eq!(render(&mut *node, 0.5, Some(&[&l, &r])), (0.75, 0.375));
    // Mono sidechain feeds both aux channels.
    assert_eq!(render(&mut *node, 0.0, Some(&[&l])), (0.25, 0.25));
    // Source removed: silent again.
    assert_eq!(render(&mut *node, 0.5, None), (0.5, 0.5));
    assert_eq!(plugin.underruns(), 0);
    plugin.deactivate(node);
}

#[test]
fn clap_sidechain_through_the_sandbox() {
    check(spawn(&clap_bundle(), CLAP_ID, PluginFormat::Clap));
}

#[test]
fn vst3_sidechain_through_the_sandbox() {
    check(spawn(
        &vst3_bundle(),
        ether_vst3::testing::EFFECT_ID,
        PluginFormat::Vst3,
    ));
}
