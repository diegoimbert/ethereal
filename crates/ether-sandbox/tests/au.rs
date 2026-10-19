//! Sandboxed AU hosting (`ether-sandbox-helper --format au`) against Apple's built-in units.
//! Owned by the `au` node.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ether_clap::testing;
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, EventKind, ProcessEvent};
use ether_core::node::ProcessContext;
use ether_core::plugin::{PluginController, PluginNode};
use ether_core::protocol::model::{ParamId, PluginFormat};
use ether_core::transport::TransportInfo;
use ether_plugin_host::PluginFormatHost;
use ether_sandbox::{SandboxOptions, SandboxedPlugin};

const LOWPASS: &str = "aufx:lpas:appl";
const DELAY: &str = "aufx:dely:appl";
const CUTOFF: ParamId = ParamId(0);
const DELAY_TIME: ParamId = ParamId(1);
const MAX: usize = 128;

fn spawn(id: &str) -> SandboxedPlugin {
    let options = SandboxOptions {
        helper: PathBuf::from(env!("CARGO_BIN_EXE_ether-sandbox-helper")),
        format: PluginFormat::Au,
        wait_budget: Some(Duration::from_secs(10)),
        ..SandboxOptions::default()
    };
    SandboxedPlugin::spawn(Path::new(id), id, &testing::instance_id(), options)
        .expect("spawn sandboxed AU")
}

fn config() -> PrepareConfig {
    PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: MAX,
        max_events_per_block: 64,
    }
}

/// Render `blocks` full blocks of a deterministic input; returns the left output.
fn render(node: &mut dyn PluginNode, blocks: usize) -> Vec<f32> {
    let transport = TransportInfo::STOPPED;
    let mut out_events = EventBuffer::with_capacity(64);
    let (mut il, mut ir) = (vec![0.0f32; MAX], vec![0.0f32; MAX]);
    let (mut ol, mut or) = (vec![0.0f32; MAX], vec![0.0f32; MAX]);
    let mut out = Vec::new();
    for b in 0..blocks {
        for i in 0..MAX {
            let t = b * MAX + i;
            il[i] = if t % 7 < 3 { 0.8 } else { -0.6 };
            ir[i] = il[i];
        }
        let events = if b == 1 {
            vec![ProcessEvent {
                offset: 0,
                kind: EventKind::Param {
                    param: CUTOFF,
                    value: 500.0,
                },
            }]
        } else {
            Vec::new()
        };
        out_events.clear();
        let inputs: [&[f32]; 2] = [&il, &ir];
        let mut outputs: [&mut [f32]; 2] = [&mut ol, &mut or];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: MAX,
            transport: &transport,
            events: &events,
            out_events: &mut out_events,
        };
        node.process(
            &mut ctx,
            &mut AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            },
        );
        out.extend_from_slice(&ol);
    }
    out
}

#[test]
fn sandboxed_au_matches_in_process_one_block_later() {
    let mut inproc = ether_au::AuFormat
        .instantiate(Path::new(LOWPASS), LOWPASS)
        .expect("in-process");
    let mut node = inproc.activate(&config()).unwrap();
    let expected = render(node.as_mut(), 12);
    inproc.deactivate(node);

    let mut sandbox = spawn(LOWPASS);
    assert_eq!(sandbox.descriptor().name, "AULowpass");
    // Ranges may depend on the sample rate (cutoff max = Nyquist-ish): compare ids/names.
    let names = |p: Vec<ether_core::protocol::devices::ParamInfo>| {
        p.into_iter().map(|p| (p.id, p.name)).collect::<Vec<_>>()
    };
    assert_eq!(names(sandbox.params()), names(inproc.params()));
    let mut node = sandbox.activate(&config()).unwrap();
    assert_eq!(node.latency(), MAX as u32);
    let got = render(node.as_mut(), 12);
    assert!(!node.is_faulted());
    sandbox.deactivate(node);

    for (t, s) in got.iter().enumerate() {
        let want = if t < MAX { 0.0 } else { expected[t - MAX] };
        assert!((s - want).abs() < 1e-5, "sample {t}: {s} vs {want}");
    }
    assert!(expected.iter().any(|s| s.abs() > 0.01));
}

#[test]
fn sandboxed_au_params_and_state() {
    let mut a = spawn(DELAY);
    a.set_param_value(DELAY_TIME, 0.2).unwrap();
    assert!((a.param_value(DELAY_TIME).unwrap() - 0.2).abs() < 1e-6);
    let state = a.save_state().unwrap();
    let mut b = spawn(DELAY);
    b.load_state(&state).unwrap();
    assert!((b.param_value(DELAY_TIME).unwrap() - 0.2).abs() < 1e-6);
}
