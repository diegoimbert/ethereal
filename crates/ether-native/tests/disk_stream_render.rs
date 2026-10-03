//! `audio-streaming` on the native bridge, rendered by the real engine on the test thread
//! (the "audio thread"), with the real controller and disk store.
//!
//! The same project is rendered twice: media decoded whole (v0.2) and streamed from disk
//! (the shared reader thread fills the caches while the test renders at ~4x real time).
//! Playing from the start, after a locate, around a transport loop and through the
//! Complex (stretched) warp path, the streamed render is bit-identical and never underruns;
//! a resampled (44.1 kHz) file matches within resampler rounding. Every block renders under
//! `assert_no_alloc` (the streaming `AudioSource::read` included).

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use assert_no_alloc::assert_no_alloc;
use common::{sine, wav};
use ether_controller::{Controller, ControllerConfig, EtherController};
use ether_core::protocol::ReplyResult;
use ether_core::protocol::message::{ClientMessage, ServerMessage};
use ether_core::protocol::model::IdGen;
use ether_core::{Engine, GarbageCollector, PrepareConfig};
use ether_native::plugins::{PluginCatalog, PluginHost};
use ether_native::rt::AudioShared;
use ether_native::store::DiskStore;
use ether_native::test_util::TempDir;
use ether_native::{DedicatedThread, LibraryRoot, NativeBridge, NativeServices};
use serde_json::{Value, json};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: u32 = 48_000;
const BLOCK: usize = 256;
/// Seconds of media (≥ STREAM_MIN_SECONDS).
const SECONDS: usize = 40;

/// Resident set size of this process, bytes (Linux).
fn rss() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    pages * 4096
}

type Ctl = EtherController<NativeBridge, NativeServices, DiskStore, DiskStore>;

struct Rig {
    ctl: Ctl,
    engine: Engine,
    gc: GarbageCollector,
    next_id: u32,
    ids: IdGen,
    now: u64,
    out: Vec<ServerMessage>,
    streamed: bool,
    _tmp: TempDir,
}

fn library_file(dir: &std::path::Path, rate: u32, seconds: usize) -> &'static str {
    let frames = rate as usize * seconds;
    // Two partials per channel: not periodic over a chunk, so misplaced chunks show.
    let l: Vec<f32> = sine(rate, 220.0, frames, 0.4)
        .iter()
        .zip(sine(rate, 1337.0, frames, 0.2))
        .map(|(a, b)| a + b)
        .collect();
    let r: Vec<f32> = sine(rate, 330.0, frames, 0.4)
        .iter()
        .zip(sine(rate, 91.0, frames, 0.3))
        .map(|(a, b)| a + b)
        .collect();
    std::fs::write(dir.join("long.wav"), wav(rate, &[l, r])).unwrap();
    "long.wav"
}

impl Rig {
    fn new(streamed: bool, file_rate: u32) -> Self {
        Self::with_length(streamed, file_rate, SECONDS)
    }

    fn with_length(streamed: bool, file_rate: u32, seconds: usize) -> Self {
        let tmp = TempDir::new("disk-stream-render");
        let projects = tmp.path().join("projects");
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        library_file(&lib, file_rate, seconds);
        let mut parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            ..Default::default()
        });
        parts
            .handle
            .set_stretcher_factory(Arc::new(ether_stretch::SignalsmithFactory::default()));
        let shared = Arc::new(AudioShared::default());
        shared.recording.set_projects_root(projects.clone());
        let mut bridge = NativeBridge::new(
            parts.handle,
            PrepareConfig {
                sample_rate: SR as f32,
                max_block_size: BLOCK,
                max_events_per_block: 256,
            },
            PluginHost::new(Arc::new(DedicatedThread::new())),
            PluginCatalog::default(),
            ether_native::plugins::instantiate_any(),
            shared,
        );
        bridge.set_disk_streaming(streamed);
        let store = DiskStore::new(
            projects,
            vec![LibraryRoot {
                id: "lib".into(),
                name: "Library".into(),
                path: lib,
            }],
        );
        let ctl = EtherController::with_config(
            bridge,
            NativeServices,
            store.clone(),
            store,
            ControllerConfig::default(),
        );
        Self {
            ctl,
            engine: parts.engine,
            gc: parts.gc,
            next_id: 1,
            ids: IdGen::new(0x5717),
            now: 1_000,
            out: Vec::new(),
            streamed,
            _tmp: tmp,
        }
    }

    fn id(&mut self) -> String {
        self.now += 1;
        self.ids.next_ulid(self.now).to_string()
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id, "gesture": null, "command": {"domain": domain, "command": command},
        }))
        .unwrap();
        self.ctl.handle(msg, &mut self.out);
        let reply = self
            .out
            .iter()
            .rev()
            .find_map(|m| match m {
                ServerMessage::Reply(r) if r.id == id => Some(r.result.clone()),
                _ => None,
            })
            .expect("reply");
        match reply {
            ReplyResult::Ok { value } => serde_json::to_value(value).unwrap(),
            ReplyResult::Err { error } => panic!("{domain} {command}: {}", error.message),
        }
    }

    fn tick(&mut self) {
        self.now += 16;
        self.out.clear();
        self.ctl.tick(self.now, &mut self.out);
        self.gc.collect();
    }

    /// Project with one audio track holding the long media at beat 0; returns the clip.
    fn setup(&mut self) -> (String, String) {
        let pid = uuid_like(&mut self.ids, self.now);
        self.ok(
            "Project",
            json!({"type": "Create", "id": pid, "name": "Stream"}),
        );
        let track = self.id();
        self.ok(
            "Track",
            json!({"type": "Create", "id": track, "kind": "Audio", "name": "Long",
                   "color": null, "parent": null, "before": null}),
        );
        let media = self.id();
        self.ok(
            "Media",
            json!({"type": "Import", "id": media, "source": {"type": "Location",
                   "location": {"type": "Library", "id": "lib"}, "path": "long.wav"}}),
        );
        let clip = self.id();
        self.ok(
            "Clip",
            json!({"type": "CreateAudio", "id": clip, "track": track, "start": 0.0,
                   "media": media}),
        );
        for _ in 0..10_000 {
            self.tick();
            if !self.ctl.media_pending() {
                break;
            }
        }
        self.tick();
        assert_eq!(
            !self.ctl.bridge.streamed_media().is_empty(),
            self.streamed,
            "streamed as configured"
        );
        (media, clip)
    }

    /// Render `seconds` of the master (left, right interleaved per block), pacing the
    /// streamed run at ~4x real time so the reader thread runs alongside.
    fn render(&mut self, seconds: f64) -> Vec<f32> {
        let frames = (seconds * SR as f64) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        let mut l = vec![0.0f32; BLOCK];
        let mut r = vec![0.0f32; BLOCK];
        let start = Instant::now();
        let mut done = 0;
        while done < frames {
            if done % (BLOCK * 8) == 0 {
                self.tick();
            }
            let engine = &mut self.engine;
            assert_no_alloc(|| {
                let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
                engine.process(&[], &mut outs, BLOCK);
            });
            out.extend_from_slice(&l);
            out.extend_from_slice(&r);
            done += BLOCK;
            if self.streamed {
                let due = Duration::from_secs_f64(done as f64 / SR as f64 / 4.0);
                if let Some(wait) = due.checked_sub(start.elapsed()) {
                    std::thread::sleep(wait);
                }
            }
        }
        out
    }

    fn underruns(&self, media: &str) -> u64 {
        let id = media.parse().unwrap();
        self.ctl
            .bridge
            .stream_cache(id)
            .map_or(0, |c| c.underruns())
    }
}

fn uuid_like(ids: &mut IdGen, now: u64) -> String {
    ids.next_project_id(now).to_string()
}

/// Run `scenario` on a fresh rig; returns the render and the stream underruns.
fn run(
    streamed: bool,
    rate: u32,
    scenario: impl Fn(&mut Rig, &str, &str) -> Vec<f32>,
) -> (Vec<f32>, u64) {
    let mut rig = Rig::new(streamed, rate);
    let (media, clip) = rig.setup();
    #[cfg(debug_assertions)]
    assert_no_alloc::reset_violation_count();
    let out = scenario(&mut rig, &media, &clip);
    #[cfg(debug_assertions)]
    assert_eq!(
        assert_no_alloc::violation_count(),
        0,
        "allocation on the audio thread"
    );
    (out, rig.underruns(&media))
}

fn play(rig: &mut Rig, seconds: f64) -> Vec<f32> {
    rig.ok("Transport", json!({"type": "Play"}));
    let out = rig.render(seconds);
    rig.ok("Transport", json!({"type": "Stop"}));
    rig.tick();
    out
}

fn compare(scenario: impl Fn(&mut Rig, &str, &str) -> Vec<f32> + Copy, what: &str) {
    let (memory, _) = run(false, SR, scenario);
    let (streamed, underruns) = run(true, SR, scenario);
    assert!(
        memory.iter().any(|v| v.abs() > 0.1),
        "{what}: silent reference"
    );
    assert_eq!(underruns, 0, "{what}: underruns");
    assert_eq!(memory.len(), streamed.len());
    let first = memory.iter().zip(&streamed).position(|(a, b)| a != b);
    assert_eq!(
        first, None,
        "{what}: streamed render differs (sample index)"
    );
}

#[test]
fn plays_from_the_start_identically() {
    compare(|rig, _, _| play(rig, 6.0), "from start");
}

#[test]
fn locate_mid_file_is_primed() {
    compare(
        |rig, _, _| {
            let mut out = play(rig, 1.0);
            // Beat 50 = 25 s (120 BPM): far outside what is cached.
            rig.ok("Transport", json!({"type": "Locate", "position": 50.0}));
            out.extend(play(rig, 2.0));
            rig.ok("Transport", json!({"type": "Locate", "position": 12.0}));
            out.extend(play(rig, 1.0));
            out
        },
        "locate",
    );
}

#[test]
fn transport_loop_wraps_without_underruns() {
    compare(
        |rig, _, _| {
            rig.ok(
                "Transport",
                json!({"type": "SetLoopRegion", "region": {"start": 60.0, "end": 64.0}}),
            );
            rig.ok(
                "Transport",
                json!({"type": "SetLoopEnabled", "enabled": true}),
            );
            rig.ok("Transport", json!({"type": "Locate", "position": 60.0}));
            rig.tick();
            // 2 s loop, three passes.
            play(rig, 6.5)
        },
        "loop",
    );
}

#[test]
fn complex_warp_streams_through_the_stretcher() {
    compare(
        |rig, _, clip| {
            rig.ok(
                "Warp",
                json!({"type": "SetWarp", "clip": clip,
                       "warp": {"enabled": true, "mode": "Complex", "source_bpm": 100.0}}),
            );
            rig.ok(
                "Clip",
                json!({"type": "SetTranspose", "id": clip, "semitones": 3.0}),
            );
            rig.tick();
            let mut out = play(rig, 3.0);
            rig.ok("Transport", json!({"type": "Locate", "position": 40.0}));
            out.extend(play(rig, 2.0));
            out
        },
        "complex warp",
    );
}

#[test]
fn resampled_media_stream_within_rounding() {
    let scenario = |rig: &mut Rig, _: &str, _: &str| {
        let mut out = play(rig, 3.0);
        rig.ok("Transport", json!({"type": "Locate", "position": 30.0}));
        out.extend(play(rig, 2.0));
        out
    };
    let (memory, _) = run(false, 44_100, scenario);
    let (streamed, underruns) = run(true, 44_100, scenario);
    assert_eq!(underruns, 0);
    let worst = memory
        .iter()
        .zip(&streamed)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-4, "max difference {worst}");
}

#[test]
fn stream_memory_is_bounded() {
    let mut rig = Rig::new(true, SR);
    let (media, _) = rig.setup();
    let cache = rig.ctl.bridge.stream_cache(media.parse().unwrap()).unwrap();
    let whole = SECONDS * SR as usize * 2 * 4;
    assert!(cache.memory_bytes() < 5 << 20, "{}", cache.memory_bytes());
    assert!(cache.memory_bytes() * 3 < whole);
}

/// `cargo test --release -p ether-native --test disk_stream_render -- --ignored --nocapture`:
/// resident memory after loading a 5-minute stereo 48 kHz file, decoded whole vs streamed.
#[test]
#[ignore = "measurement (prints RSS); slow in debug"]
fn measure_memory_of_a_five_minute_file() {
    for streamed in [true, false] {
        let before = rss();
        let mut rig = Rig::with_length(streamed, SR, 300);
        let file = rss();
        let (_media, _) = rig.setup();
        // Let the reader thread settle and the GC run.
        for _ in 0..20 {
            rig.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
        let after = rss();
        println!(
            "5 min stereo, {}: +{:.1} MB resident after load (test setup +{:.1} MB)",
            if streamed { "streamed" } else { "whole-file" },
            after.saturating_sub(file) as f64 / 1e6,
            file.saturating_sub(before) as f64 / 1e6,
        );
        drop(rig);
    }
}
