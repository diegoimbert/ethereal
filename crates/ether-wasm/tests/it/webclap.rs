//! WebCLAP over the web pipeline (`ether_wasm::webclap`): the real `EtherController`
//! drives a `WebBridge`, the Worklet's `EngineHost` renders on the other side of
//! heap-backed rings, and a fake guest stands in for the JS host glue (which has its own
//! tests against a real plugin compiled to wasm: `apps/web/src/engine/webclap/*.test.ts`).
//!
//! The fake plugin is a gain (param 1, plain 0..2) applied sample-accurately at its
//! events' offsets, with an optional pure delay reported as latency, a state (the gain,
//! 8 bytes) and `SLEEP` on silent input.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, ControllerConfig, EngineBridge, EtherController, HostServices};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::ring::HeapMemory;
use ether_wasm::webclap::catalog::Catalog;
use ether_wasm::webclap::guest::{self, Guest, GuestNote, GuestSpec, status};
use ether_wasm::webclap::node::{EventRec, Exchange, kind};
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM, tap_clock};

const SR: u32 = 48_000;
const T0: u64 = 1_750_000_000_000;
const TICK_MS: u64 = 16;
const TICK_FRAMES: usize = SR as usize * TICK_MS as usize / 1000;
const GAIN: u32 = 1;
const DELAY: u32 = 64;

// ─── The fake guest ──────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Inst {
    plugin: String,
    gain: f64,
    delay: Vec<[f32; 2]>,
    pos: usize,
    calls: usize,
    created_with: Option<Vec<u8>>,
}

#[derive(Default)]
struct Fake {
    next: u32,
    insts: BTreeMap<u32, Inst>,
    notes: Vec<(u32, GuestNote)>,
}

#[derive(Clone, Default)]
struct FakeGuest(Rc<RefCell<Fake>>);

impl Guest for FakeGuest {
    fn create(&self, spec: &GuestSpec<'_>) -> u32 {
        let mut f = self.0.borrow_mut();
        f.next += 1;
        let gain = spec
            .state
            .map(|s| f64::from_le_bytes(s.try_into().expect("8 bytes")))
            .unwrap_or(1.0);
        let delay = if spec.plugin_id == "test.delay" {
            DELAY as usize
        } else {
            0
        };
        let h = f.next;
        f.insts.insert(
            h,
            Inst {
                plugin: spec.plugin_id.to_string(),
                gain,
                delay: vec![[0.0; 2]; delay],
                created_with: spec.state.map(<[u8]>::to_vec),
                ..Inst::default()
            },
        );
        h
    }

    fn process(&self, handle: u32, x: &mut Exchange) -> i32 {
        let mut f = self.0.borrow_mut();
        let Some(inst) = f.insts.get_mut(&handle) else {
            return status::FAULTED;
        };
        inst.calls += 1;
        let frames = x.frames as usize;
        // SAFETY: the node's buffers, alive for this call (as the JS host reads them).
        let events: &[EventRec] =
            unsafe { std::slice::from_raw_parts(x.events, x.n_events as usize) };
        let mut e = 0;
        let mut silent = true;
        for i in 0..frames {
            while e < events.len() && events[e].offset as usize <= i {
                if events[e].kind == kind::PARAM && events[e].id == GAIN {
                    inst.gain = events[e].value;
                }
                e += 1;
            }
            for c in 0..x.out_channels as usize {
                let input = if c < x.in_channels as usize {
                    unsafe { *x.inputs[c].add(i) }
                } else {
                    0.0
                };
                silent &= input == 0.0;
                let mut s = input * inst.gain as f32;
                if !inst.delay.is_empty() {
                    let slot = &mut inst.delay[inst.pos][c];
                    std::mem::swap(slot, &mut s);
                }
                unsafe { *x.outputs[c].add(i) = s };
            }
            if !inst.delay.is_empty() {
                inst.pos = (inst.pos + 1) % inst.delay.len();
            }
        }
        if silent && inst.delay.is_empty() {
            status::SLEEP
        } else {
            status::CONTINUE
        }
    }

    fn ready(&self, handle: u32) -> Result<bool, String> {
        Ok(self.0.borrow().insts.contains_key(&handle))
    }

    fn latency(&self, handle: u32) -> u32 {
        self.0
            .borrow()
            .insts
            .get(&handle)
            .map_or(0, |i| i.delay.len() as u32)
    }

    fn take_wake(&self, _handle: u32) -> bool {
        false
    }

    fn reset(&self, _handle: u32) {}

    fn destroy(&self, handle: u32) {
        self.0.borrow_mut().insts.remove(&handle);
    }

    fn save_state(&self, handle: u32) -> Result<Option<Vec<u8>>, String> {
        let f = self.0.borrow();
        let inst = f.insts.get(&handle).ok_or("gone")?;
        if inst.plugin == "test.huge" {
            return Ok(Some(vec![
                0;
                ether_wasm::webclap::worklet::MAX_STATE_BYTES + 1
            ]));
        }
        Ok(Some(inst.gain.to_le_bytes().to_vec()))
    }

    fn param_values(&self, handle: u32) -> Option<Vec<(u32, f64)>> {
        let f = self.0.borrow();
        f.insts.get(&handle).map(|i| vec![(GAIN, i.gain)])
    }

    fn take_notifications(&self, out: &mut Vec<(u32, GuestNote)>) {
        out.append(&mut self.0.borrow_mut().notes);
    }
}

impl FakeGuest {
    fn handle_of(&self, plugin: &str) -> u32 {
        *self
            .0
            .borrow()
            .insts
            .iter()
            .find(|(_, i)| i.plugin == plugin)
            .expect("no instance")
            .0
    }
    fn calls(&self, plugin: &str) -> usize {
        let h = self.handle_of(plugin);
        self.0.borrow().insts[&h].calls
    }
}

const CATALOG: &str = r#"[{ "module": "m1", "plugins": [
  { "id": "test.gain", "name": "Test Gain", "vendor": "Ethereal", "features": ["audio-effect"],
    "params": [{ "id": 1, "flags": 32, "name": "gain", "min": 0, "max": 2, "default": 1 }] },
  { "id": "test.delay", "name": "Test Delay", "features": ["audio-effect"],
    "params": [{ "id": 1, "flags": 32, "name": "gain", "min": 0, "max": 2, "default": 1 }] },
  { "id": "test.huge", "name": "Test Huge State", "features": ["audio-effect"],
    "params": [{ "id": 1, "flags": 32, "name": "gain", "min": 0, "max": 2, "default": 1 }] }
]}]"#;

// ─── Driver ──────────────────────────────────────────────────────────────────────────────

struct TestHost {
    now: u64,
}

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn random_seed(&mut self) -> u64 {
        0x5eed
    }
}

struct Drive {
    ctl: EtherController<WebBridge<HeapMemory>, TestHost, MemoryStore, MemoryLibrary>,
    host: Rc<RefCell<EngineHost<HeapMemory>>>,
    shared: bridge::Shared<HeapMemory>,
    guest: FakeGuest,
    ids: IdGen,
    next: u32,
    project: ProjectId,
    track: TrackId,
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    b
}

impl Drive {
    fn new() -> Self {
        let guest = FakeGuest::default();
        guest::set_guest(Some(Rc::new(guest.clone())));
        let mut lib = MemoryLibrary::new();
        lib.add_root("lib", "Library");
        // Half a second of DC at 0.5 after 100 ms of silence.
        let mut dc = vec![0.0f32; 4800];
        dc.extend(std::iter::repeat_n(0.5, SR as usize / 2));
        lib.add_file("lib", "dc.wav", wav(&dc));
        let control = HeapMemory::new(1 << 22);
        let reports = HeapMemory::new(1 << 16);
        let shared = bridge::shared(control.clone(), reports.clone());
        shared.borrow_mut().webclap.catalog = Catalog::parse(CATALOG).unwrap();
        let host = Rc::new(RefCell::new(EngineHost::new(SR, control, reports)));
        let mut web = WebBridge::new(shared.clone());
        // The Worker's wait: the Worklet renders meanwhile.
        let h = host.clone();
        web.set_webclap_waiter(Box::new(move |_| h.borrow_mut().render(RENDER_QUANTUM)));
        let mut ids = IdGen::new(7);
        let track = ids.next(T0);
        let mut d = Self {
            ctl: EtherController::with_config(
                web,
                TestHost { now: T0 },
                MemoryStore::new(),
                lib,
                ControllerConfig {
                    engine_sample_rate: SR,
                    ..Default::default()
                },
            ),
            host,
            shared,
            guest,
            ids,
            next: 1,
            project: ProjectId(Default::default()),
            track,
        };
        d.project = d.ids.next_project_id(T0);
        let project = d.project;
        d.ok(Command::Project(ProjectCommand::Create {
            id: project,
            name: "Song".into(),
        }));
        let media: MediaId = d.ids.next(T0);
        d.ok(Command::Media(MediaCommand::Import {
            id: media,
            source: MediaSource::Location {
                location: BrowseLocation::Library { id: "lib".into() },
                path: "dc.wav".into(),
            },
        }));
        d.ok(Command::Track(TrackCommand::Create {
            id: track,
            kind: TrackKind::Audio,
            name: Some("A".into()),
            color: None,
            parent: None,
            before: None,
        }));
        let clip = d.ids.next(T0);
        d.ok(Command::Clip(ClipCommand::CreateAudio {
            id: clip,
            track,
            start: Beats(0.0),
            media,
        }));
        for _ in 0..10_000 {
            d.tick();
            if !d.ctl.media_pending() {
                break;
            }
        }
        d
    }

    fn try_cmd(&mut self, command: Command) -> Result<ReplyValue, CommandError> {
        let id = self.next;
        self.next += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture: None,
                command,
            },
            &mut out,
        );
        for m in &out {
            if let ServerMessage::Reply(r) = m
                && r.id == id
            {
                return match &r.result {
                    ReplyResult::Ok { value } => Ok(value.clone()),
                    ReplyResult::Err { error } => Err(error.clone()),
                };
            }
        }
        panic!("no reply")
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        self.try_cmd(command)
            .unwrap_or_else(|e| panic!("command failed: {e:?}"))
    }

    fn insert(&mut self, plugin: &str, format: PluginFormat) -> Result<DeviceId, CommandError> {
        let id = self.ids.next(T0);
        let track = self.track;
        self.try_cmd(Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Plugin {
                plugin_id: plugin.into(),
                sandboxed: None,
                format: Some(format),
                name: None,
            },
            before: None,
        }))?;
        Ok(id)
    }

    fn tick(&mut self) {
        self.ctl.host.now += TICK_MS;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut Vec::new());
    }

    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames);
        while out.len() < frames {
            let n = RENDER_QUANTUM.min(frames - out.len());
            let mut host = self.host.borrow_mut();
            host.render(n);
            out.extend_from_slice(&host.output(0)[..n]);
        }
        out
    }

    fn idle(&mut self) {
        for _ in 0..30 {
            self.render(TICK_FRAMES);
            self.tick();
        }
    }

    /// Play from beat 0 for `frames`; returns the left output.
    fn play(&mut self, frames: usize) -> Vec<f32> {
        self.ok(Command::Transport(TransportCommand::Stop));
        self.ok(Command::Transport(TransportCommand::Locate {
            position: Beats(0.0),
        }));
        self.tick();
        self.render(TICK_FRAMES);
        self.ok(Command::Transport(TransportCommand::Play));
        self.tick();
        let out = self.render(frames);
        self.ok(Command::Transport(TransportCommand::Stop));
        self.tick();
        self.render(TICK_FRAMES);
        out
    }

    fn device(&self, id: DeviceId) -> Device {
        self.ctl.doc.as_ref().unwrap().project.devices[&id].clone()
    }

    fn errors(&self) -> Vec<String> {
        self.shared.borrow().errors.clone()
    }
}

/// The steady level of a played DC clip (well after its onset).
fn level(out: &[f32]) -> f32 {
    out[out.len() - 2000]
}

#[test]
fn a_webclap_effect_processes_and_follows_its_params() {
    let mut d = Drive::new();
    let dry = level(&d.play(SR as usize / 4));
    assert!((dry - 0.5).abs() < 0.01, "dry {dry}");

    let fx = d.insert("test.gain", PluginFormat::Webclap).unwrap();
    d.idle();
    let dev = d.device(fx);
    let DeviceKind::Plugin { plugin } = &dev.kind else {
        panic!("not a plugin")
    };
    assert_eq!(plugin.format, PluginFormat::Webclap);
    assert_eq!(plugin.name, "Test Gain");
    // Mirrored from the instance after creation.
    assert_eq!(dev.params.get(&ParamId(GAIN)), Some(&1.0));
    let wet = level(&d.play(SR as usize / 4));
    assert!(
        (wet - dry).abs() < 1e-4,
        "unity gain changed the level: {wet}"
    );

    d.ok(Command::Device(DeviceCommand::SetParam {
        device: fx,
        param: ParamId(GAIN),
        value: 0.5,
    }));
    d.idle();
    let half = level(&d.play(SR as usize / 4));
    assert!((half - dry * 0.5).abs() < 1e-3, "gain 0.5 → {half}");
    // The descriptor comes from the catalog.
    let desc = d.ctl.bridge.descriptor(fx).expect("descriptor");
    assert_eq!(desc.params.len(), 1);
    assert_eq!(desc.params[0].name, "gain");
    assert!(d.errors().is_empty(), "{:?}", d.errors());
}

#[test]
fn state_is_read_from_the_worklet_and_restored_on_reopen() {
    let mut d = Drive::new();
    let fx = d.insert("test.gain", PluginFormat::Webclap).unwrap();
    d.idle();
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: fx,
        param: ParamId(GAIN),
        value: 0.25,
    }));
    d.idle();
    // A blocking round trip to the Worklet's instance.
    let state = d.ctl.bridge.plugin_state(fx).unwrap().expect("a state");
    assert_eq!(state.0, 0.25f64.to_le_bytes().to_vec());

    d.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let DeviceKind::Plugin { plugin } = d.device(fx).kind else {
        panic!()
    };
    let _ = plugin;
    let project = d.project;
    // Reopen: the new instance is created with the saved state.
    let other = d.ids.next_project_id(T0);
    d.ok(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    }));
    d.ok(Command::Project(ProjectCommand::Open { id: project }));
    d.idle();
    let h = d.guest.handle_of("test.gain");
    let created = d.guest.0.borrow().insts[&h].created_with.clone();
    assert_eq!(created, Some(0.25f64.to_le_bytes().to_vec()));
    assert_eq!(d.device(fx).params.get(&ParamId(GAIN)), Some(&0.25));
}

/// A state over the cap isn't passed on: the save fails (the worklet answers `Failed`)
/// and later requests still get answered.
#[test]
fn a_state_over_the_cap_fails_the_save() {
    let mut d = Drive::new();
    let huge = d.insert("test.huge", PluginFormat::Webclap).unwrap();
    let fx = d.insert("test.gain", PluginFormat::Webclap).unwrap();
    d.idle();
    let err = d.ctl.bridge.plugin_state(huge).unwrap_err();
    assert!(format!("{err:?}").contains("larger than"), "{err:?}");
    let state = d.ctl.bridge.plugin_state(fx).unwrap().expect("a state");
    assert_eq!(state.0, 1.0f64.to_le_bytes().to_vec());
}

#[test]
fn plugin_edits_become_document_edits() {
    let mut d = Drive::new();
    let fx = d.insert("test.gain", PluginFormat::Webclap).unwrap();
    d.idle();
    let h = d.guest.handle_of("test.gain");
    d.guest.0.borrow_mut().notes.extend([
        (h, GuestNote::GestureBegin { id: GAIN }),
        (
            h,
            GuestNote::ParamEdited {
                id: GAIN,
                value: 1.5,
            },
        ),
        (h, GuestNote::GestureEnd { id: GAIN }),
    ]);
    d.idle();
    assert_eq!(d.device(fx).params.get(&ParamId(GAIN)), Some(&1.5));
    // Undoable like any edit.
    d.ok(Command::Edit(
        ether_core::protocol::project::EditCommand::Undo,
    ));
    d.idle();
    assert_ne!(d.device(fx).params.get(&ParamId(GAIN)), Some(&1.5));
}

#[test]
fn latency_is_compensated() {
    let mut d = Drive::new();
    d.insert("test.delay", PluginFormat::Webclap).unwrap();
    d.idle();
    let lat = d.host.borrow().tap_clock()[tap_clock::LATENCY] as u32;
    assert_eq!(lat, DELAY);
}

#[test]
fn a_silent_effect_sleeps() {
    let mut d = Drive::new();
    d.insert("test.gain", PluginFormat::Webclap).unwrap();
    d.idle();
    // Stopped transport: silent input, the plugin said SLEEP.
    let before = d.guest.calls("test.gain");
    d.render(SR as usize / 10);
    let after = d.guest.calls("test.gain");
    assert!(
        after - before <= 1,
        "called {} times while asleep",
        after - before
    );
    // Playing wakes it.
    d.play(SR as usize / 4);
    assert!(d.guest.calls("test.gain") > after + 10);
}

#[test]
fn missing_and_native_plugins_fail_without_crashing() {
    let mut d = Drive::new();
    // Inserting needs the plugin: a WebCLAP that isn't installed, a native format.
    let e = d
        .insert("not.installed", PluginFormat::Webclap)
        .unwrap_err();
    assert!(e.message.contains("is not installed"), "{}", e.message);
    let e = d.insert("com.u-he.diva", PluginFormat::Clap).unwrap_err();
    assert!(
        e.message.contains("not available in the browser"),
        "{}",
        e.message
    );
    let dry = level(&d.play(SR as usize / 4));
    assert!((dry - 0.5).abs() < 0.01);
}

#[test]
fn offline_renders_use_a_fresh_instance() {
    let mut d = Drive::new();
    let fx = d.insert("test.gain", PluginFormat::Webclap).unwrap();
    d.idle();
    let DeviceKind::Plugin { plugin } = d.device(fx).kind else {
        panic!()
    };
    let state = Base64Bytes(0.75f64.to_le_bytes().to_vec());
    let node = d
        .ctl
        .bridge
        .create_offline_plugin(fx, &plugin, Some(&state), SR)
        .expect("offline node");
    let n = d.guest.0.borrow().insts.len();
    assert_eq!(n, 2, "a second instance");
    drop(node);
    assert_eq!(
        d.guest.0.borrow().insts.len(),
        1,
        "dropping it destroys the instance"
    );
}
