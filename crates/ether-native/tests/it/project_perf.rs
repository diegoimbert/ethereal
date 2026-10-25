//! A stored project end to end on the native stack (the real controller, the native bridge,
//! the engine with its worker pool on the paced `null` backend): open time until the load
//! ends, then playback CPU (the callback's load and worst time, late callbacks) and the
//! process's memory. For profiling real projects, which stay outside the repo:
//!
//! ```text
//! ETHER_PERF_PROJECT=/path/to/project.ether \
//!   cargo test --release -p ether-native --test it project_perf:: -- --ignored --nocapture
//! ```
//! Plugins the machine doesn't have stay placeholders (bypassed); missing media are silent.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_controller::{Controller, EtherController};
use ether_core::config::PrepareConfig;
use ether_core::protocol::model::file;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_native::audio::{AudioBackendKind, AudioOutput, AudioSettings};
use ether_native::plugins::{Instantiate, PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::workers::{PoolOptions, WorkerPool, default_workers};
use ether_native::{DedicatedThread, NativeBridge, NativeServices};
use serde_json::json;

const RATE: u32 = 48_000;
const BLOCK: usize = 256;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Resident memory of this process, MB (Linux; `None` elsewhere).
fn rss_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096.0 / 1e6)
}

#[test]
#[ignore = "perf harness: ETHER_PERF_PROJECT=<.ether>, release, --ignored --nocapture"]
fn stored_project_open_and_playback() {
    let Ok(path) = std::env::var("ETHER_PERF_PROJECT") else {
        return;
    };
    let json_text = std::fs::read_to_string(&path).expect("ETHER_PERF_PROJECT is readable");
    let project = file::load(&json_text).expect("a project file");
    let rss0 = rss_mb();

    let workers = default_workers();
    let mut parts = ether_core::create(ether_core::EngineConfig {
        sample_rate: RATE,
        max_block_size: BLOCK,
        worker_threads: workers,
        ..Default::default()
    });
    if workers > 0 {
        parts.engine.set_executor(Box::new(WorkerPool::with_options(
            workers,
            PoolOptions {
                period: Duration::from_secs_f64(BLOCK as f64 / RATE as f64),
                ..Default::default()
            },
        )));
    }
    let shared = Arc::new(AudioShared::default());
    let output = AudioOutput::start(
        Box::new(parts.engine),
        &AudioSettings {
            backend: AudioBackendKind::Null,
            sample_rate: Some(RATE),
            max_block_size: BLOCK,
            buffer_size: Some(BLOCK as u32),
            ..Default::default()
        },
        shared.clone(),
    )
    .map_err(|(e, _)| e)
    .expect("null backend starts");
    let no_plugins: Instantiate = Arc::new(|_, _, _| {
        Err(ether_core::plugin::PluginError::NotFound(
            "not on this machine".into(),
        ))
    });
    let bridge = NativeBridge::new(
        parts.handle,
        PrepareConfig {
            sample_rate: RATE as f32,
            max_block_size: BLOCK,
            max_events_per_block: 256,
        },
        PluginHost::new(Arc::new(DedicatedThread::new())),
        PluginCatalog::default(),
        no_plugins,
        shared.clone(),
    );
    let mut store = MemoryStore::new();
    store.create(project.id).unwrap();
    store.save(project.id, &json_text).unwrap();
    let mut ctl = EtherController::new(
        bridge,
        NativeServices::default(),
        store,
        MemoryLibrary::new(),
    );
    let mut gc = parts.gc;
    let mut out: Vec<ServerMessage> = Vec::new();
    let send = |ctl: &mut EtherController<_, _, _, _>,
                out: &mut Vec<ServerMessage>,
                id: u32,
                domain: &str,
                command: serde_json::Value| {
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id, "gesture": null, "command": {"domain": domain, "command": command},
        }))
        .unwrap();
        ctl.handle(msg, out);
        out.clear();
    };

    let t = Instant::now();
    send(
        &mut ctl,
        &mut out,
        1,
        "Project",
        json!({"type": "OpenMirrored", "id": project.id.to_string()}),
    );
    let open_handled = t.elapsed();
    let mut ticks = 0;
    while ctl.loading.active() && t.elapsed() < Duration::from_secs(60) {
        ctl.tick(now(), &mut out);
        out.clear();
        gc.collect();
        std::thread::sleep(Duration::from_millis(16));
        ticks += 1;
    }
    let loaded = t.elapsed();
    let rss1 = rss_mb();

    send(&mut ctl, &mut out, 2, "Transport", json!({"type": "Play"}));
    // Settle, then measure.
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_secs(1) {
        ctl.tick(now(), &mut out);
        out.clear();
        gc.collect();
        std::thread::sleep(Duration::from_millis(16));
    }
    shared.overruns.store(0, Ordering::Relaxed);
    shared.worst_callback_us.store(0, Ordering::Relaxed);
    let mut loads = Vec::new();
    let play = Instant::now();
    while play.elapsed() < Duration::from_secs(5) {
        ctl.tick(now(), &mut out);
        out.clear();
        gc.collect();
        loads.push(shared.cpu_load());
        std::thread::sleep(Duration::from_millis(16));
    }
    loads.sort_by(f32::total_cmp);
    let p = |q: f64| loads[((loads.len() - 1) as f64 * q) as usize];
    let budget_us = BLOCK as f64 * 1e6 / RATE as f64;
    println!(
        "project_perf {}: {} tracks, {} devices, {} notes, {} clips; open handled in {:.0} ms, load ended after {:.0} ms ({ticks} ticks); \
         playing ({workers} workers, {BLOCK} frames, {budget_us:.0} µs budget): callback load p50 {:.1}% p95 {:.1}%, worst callback {} µs, late callbacks {} in 5 s; \
         RSS {:.0} MB before, {:.0} MB loaded, {:.0} MB playing",
        std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        project.tracks.len(),
        project.devices.len(),
        project.notes.len(),
        project.clips.len(),
        open_handled.as_secs_f64() * 1e3,
        loaded.as_secs_f64() * 1e3,
        p(0.5) * 100.0,
        p(0.95) * 100.0,
        shared.worst_callback_us.load(Ordering::Relaxed),
        shared.overruns.load(Ordering::Relaxed),
        rss0.unwrap_or(0.0),
        rss1.unwrap_or(0.0),
        rss_mb().unwrap_or(0.0),
    );
    drop(ctl);
    let _ = output.stop();
}
