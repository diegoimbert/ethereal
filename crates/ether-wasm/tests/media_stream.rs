//! `audio-streaming` on the web host, natively: the real controller + `WebBridge` (the
//! Worker) streaming a long media from the OPFS fake by byte ranges, the `EngineHost` (the
//! worklet) rendering from its chunk cache, connected by heap-backed rings.
//!
//! The same session is rendered with streaming and with the whole-file load: playing from
//! the start, after a locate and around a transport loop the output is bit-identical and
//! no underrun is reported.

use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::media::MediaEvent;
use ether_core::protocol::message::{ClientMessage, Event, ReplyResult, ServerMessage};
use ether_core::protocol::model::{IdGen, MediaId};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::media_stream::{RangeFs, range_opener};
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{Fs, MemFs, WebLibrary, WebStore};
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};
use serde_json::{Value, json};

const SR: u32 = 48_000;
const SECONDS: usize = 40;

struct TestHost;

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        1_750_000_000_000
    }
    fn random_seed(&mut self) -> u64 {
        42
    }
}

/// Ranged reads over the OPFS fake.
#[derive(Clone)]
struct MemRange(MemFs);

impl RangeFs for MemRange {
    fn size(&mut self, path: &str) -> Result<u64, String> {
        Ok(self.0.read(path).map_err(|e| e.to_string())?.len() as u64)
    }
    fn read_range(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        let all = self.0.read(path).map_err(|e| e.to_string())?;
        let a = (offset as usize).min(all.len());
        let b = (a + len).min(all.len());
        Ok(all[a..b].to_vec())
    }
}

fn wav(rate: u32, channels: &[Vec<f32>]) -> Vec<u8> {
    let n_ch = channels.len() as u16;
    let frames = channels[0].len();
    let data_len = (frames * n_ch as usize * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&n_ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * n_ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(n_ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for ch in channels {
            b.extend_from_slice(&((ch[i].clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    b
}

fn signal(hz: [f32; 2], frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = i as f32 / SR as f32;
            0.4 * (std::f32::consts::TAU * hz[0] * t).sin()
                + 0.2 * (std::f32::consts::TAU * hz[1] * t).sin()
        })
        .collect()
}

type Ctl = EtherController<WebBridge<HeapMemory>, TestHost, WebStore<MemFs>, WebLibrary<MemFs>>;

struct Rig {
    ctl: Ctl,
    engine: EngineHost<HeapMemory>,
    shared: bridge::Shared<HeapMemory>,
    next: u32,
    ids: IdGen,
    now: u64,
    events: Vec<Event>,
}

impl Rig {
    fn new(streamed: bool) -> Self {
        let control = HeapMemory::new(4 << 20);
        let reports = HeapMemory::new(256 << 10);
        let shared = bridge::shared(control.clone(), reports.clone());
        let mut fs = MemFs::new();
        let frames = SR as usize * SECONDS;
        fs.write(
            "library/long.wav",
            &wav(
                SR,
                &[
                    signal([220.0, 1337.0], frames),
                    signal([330.0, 91.0], frames),
                ],
            ),
        )
        .unwrap();
        let mut bridge = WebBridge::new(shared.clone());
        if streamed {
            bridge.set_stream_opener(range_opener(MemRange(fs.clone())));
        }
        let ctl = EtherController::with_config(
            bridge,
            TestHost,
            WebStore::new(fs.clone()),
            WebLibrary::new(fs),
            ControllerConfig {
                engine_sample_rate: SR,
                ..ControllerConfig::default()
            },
        );
        Self {
            ctl,
            engine: EngineHost::new(SR, control, reports),
            shared,
            next: 1,
            ids: IdGen::new(0x3eb),
            now: 1_000,
            events: Vec::new(),
        }
    }

    fn id(&mut self) -> String {
        self.now += 1;
        self.ids.next_ulid(self.now).to_string()
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id, "gesture": null, "command": {"domain": domain, "command": command},
        }))
        .unwrap();
        let mut out = Vec::new();
        self.ctl.handle(msg, &mut out);
        match out.last() {
            Some(ServerMessage::Reply(r)) => match &r.result {
                ReplyResult::Ok { value } => serde_json::to_value(value).unwrap(),
                ReplyResult::Err { error } => panic!("{domain} {command}: {}", error.message),
            },
            other => panic!("{other:?}"),
        }
    }

    fn tick(&mut self) {
        self.now += 16;
        let mut out = Vec::new();
        self.ctl.tick(self.now, &mut out);
        for m in out {
            if let ServerMessage::Event(e) = m {
                self.events.push(e);
            }
        }
    }

    /// Render `seconds` (stereo interleaved per quantum); the Worker ticks every 8 quanta
    /// (~21 ms, about its real rate).
    fn render(&mut self, seconds: f64) -> Vec<f32> {
        let quanta = (seconds * SR as f64 / RENDER_QUANTUM as f64) as usize;
        let mut out = Vec::with_capacity(quanta * RENDER_QUANTUM * 2);
        for q in 0..quanta {
            if q % 8 == 0 {
                self.tick();
            }
            self.engine.render(RENDER_QUANTUM);
            out.extend_from_slice(self.engine.output(0));
            out.extend_from_slice(self.engine.output(1));
        }
        out
    }

    fn setup(&mut self) -> MediaId {
        let pid = self.ids.next_project_id(self.now).to_string();
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
                   "location": {"type": "Library", "id": "browser"}, "path": "long.wav"}}),
        );
        let clip = self.id();
        self.ok(
            "Clip",
            json!({"type": "CreateAudio", "id": clip, "track": track, "start": 0.0,
                   "media": media}),
        );
        for _ in 0..10_000 {
            self.tick();
            self.engine.render(RENDER_QUANTUM);
            if !self.ctl.media_pending() {
                break;
            }
        }
        // Let the worklet apply everything queued.
        self.render(0.2);
        media.parse().unwrap()
    }

    fn play(&mut self, seconds: f64) -> Vec<f32> {
        self.ok("Transport", json!({"type": "Play"}));
        let out = self.render(seconds);
        self.ok("Transport", json!({"type": "Stop"}));
        self.render(0.05);
        out
    }

    fn underrun_reports(&self) -> u32 {
        self.events
            .iter()
            .map(|e| match e {
                Event::Media {
                    event: MediaEvent::StreamUnderruns { count },
                } => *count,
                _ => 0,
            })
            .sum()
    }
}

fn session(rig: &mut Rig) -> Vec<f32> {
    let mut out = rig.play(4.0);
    // Beat 50 = 25 s: far from anything cached.
    rig.ok("Transport", json!({"type": "Locate", "position": 50.0}));
    // The primed chunks travel ahead of the locate in the ring: a few quanta later than a
    // whole-file session; let both settle so the renders line up.
    rig.render(0.05);
    out.extend(rig.play(2.0));
    rig.ok(
        "Transport",
        json!({"type": "SetLoopRegion", "region": {"start": 60.0, "end": 64.0}}),
    );
    rig.ok(
        "Transport",
        json!({"type": "SetLoopEnabled", "enabled": true}),
    );
    rig.ok("Transport", json!({"type": "Locate", "position": 60.0}));
    rig.render(0.05);
    out.extend(rig.play(5.0));
    out
}

#[test]
fn web_streaming_renders_like_the_whole_file() {
    let mut memory = Rig::new(false);
    let m = memory.setup();
    assert!(!memory.ctl.bridge.is_streamed(m));
    let reference = session(&mut memory);

    let mut streamed = Rig::new(true);
    let m = streamed.setup();
    assert!(
        streamed.ctl.bridge.is_streamed(m),
        "long media stream on the web"
    );
    let got = session(&mut streamed);

    assert!(reference.iter().any(|v| v.abs() > 0.1), "silent reference");
    assert_eq!(streamed.underrun_reports(), 0, "underruns reported");
    assert_eq!(reference.len(), got.len());
    let first = reference.iter().zip(&got).position(|(a, b)| a != b);
    assert_eq!(first, None, "streamed render differs (sample index)");
    assert!(
        streamed.shared.borrow().errors.is_empty(),
        "{:?}",
        streamed.shared.borrow().errors
    );
}
