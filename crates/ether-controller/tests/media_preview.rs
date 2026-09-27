//! `media-preview`: `Media::{Preview, StopPreview}` through the controller with injected
//! media and a bridge that records preview calls and reports natural ends by id.
//! Covers start (decode + resample to the engine rate), natural end, stop, replace, the
//! replace-vs-natural-end race, bounded work per tick, the length cap, the decode cache,
//! project sources, failures and unsupported hosts.

mod common;

use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

const SR: u32 = 44_100;
const ENGINE_SR: u32 = 48_000;

/// A preview call as the engine saw it.
#[derive(Debug, Clone, PartialEq)]
enum PreviewCall {
    /// `(id, frames at the engine rate, channels, sample rate)`
    Play(u64, usize, usize, u32),
    Stop,
}

/// `FakeBridge` plus preview support: records calls; `ended` is reported by the next poll.
#[derive(Default)]
struct PreviewBridge {
    inner: FakeBridge,
    previews: Vec<PreviewCall>,
    ended: Option<u64>,
    fail_play: bool,
}

impl EngineBridge for PreviewBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_builtin(device, kind, params)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.inner.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.inner.load_media(media, audio)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.inner.unload_media(media)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.inner.publish(graph)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.inner.set_param(change)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.inner.transport(control)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.inner.poll(out);
        out.preview_ended = self.ended.take();
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn preview(
        &mut self,
        id: u64,
        audio: Option<Arc<DecodedAudio>>,
        _gain: f32,
    ) -> Result<(), BridgeError> {
        match audio {
            Some(a) => {
                if self.fail_play {
                    return Err(BridgeError::Other("voice unavailable".into()));
                }
                self.previews.push(PreviewCall::Play(
                    id,
                    a.frames(),
                    a.channels.len(),
                    a.sample_rate,
                ))
            }
            None => self.previews.push(PreviewCall::Stop),
        }
        Ok(())
    }
}

type Ctl = EtherController<PreviewBridge, FakeHost, MemoryStore, MemoryLibrary>;

struct H {
    ctl: Ctl,
    ids: IdGen,
    next: u32,
}

impl H {
    fn new(lib: MemoryLibrary, config: ControllerConfig) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        Self {
            ctl: EtherController::with_config(
                PreviewBridge::default(),
                FakeHost { now: T0 },
                store,
                lib,
                config,
            ),
            ids: IdGen::new(7),
            next: 1,
        }
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
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
        out
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        self.ctl.tick(T0, &mut out);
        out
    }

    fn preview(&mut self, source: MediaSource) -> Vec<ServerMessage> {
        self.send(Command::Media(MediaCommand::Preview { source }))
    }

    fn stop(&mut self) -> Vec<ServerMessage> {
        self.send(Command::Media(MediaCommand::StopPreview))
    }

    fn create_project(&mut self) {
        let id = self.ids.next_project_id(T0);
        ok(&self.send(Command::Project(ProjectCommand::Create {
            id,
            name: "Preview".into(),
        })));
    }

    fn calls(&mut self) -> Vec<PreviewCall> {
        std::mem::take(&mut self.ctl.bridge.previews)
    }
}

fn lib_source(path: &str) -> MediaSource {
    MediaSource::Location {
        location: BrowseLocation::Library { id: "lib".into() },
        path: path.into(),
    }
}

/// Preview events (`Started(path)` / `Ended(path, reason)`), in order.
#[derive(Debug, PartialEq)]
enum Ev {
    Started(String),
    Ended(String, PreviewEndReason),
}

fn name(source: &MediaSource) -> String {
    match source {
        MediaSource::Location { path, .. } => path.clone(),
        MediaSource::Project { media } => format!("project:{media}"),
        MediaSource::Upload { upload } => upload.clone(),
    }
}

fn preview_events(out: &[ServerMessage]) -> Vec<Ev> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Media {
                event: MediaEvent::PreviewStarted { source },
            } => Some(Ev::Started(name(&source))),
            Event::Media {
                event: MediaEvent::PreviewEnded { source, reason },
            } => Some(Ev::Ended(name(&source), reason)),
            _ => None,
        })
        .collect()
}

fn started(p: &str) -> Ev {
    Ev::Started(p.into())
}

fn ended(p: &str, reason: PreviewEndReason) -> Ev {
    Ev::Ended(p.into(), reason)
}

/// Library with a 0.5 s stereo kick, a 0.25 s mono snare, a 3 s pad and a text file.
fn library() -> MemoryLibrary {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let kick = [
        sine(SR, 60.0, SR as usize / 2, 0.8),
        sine(SR, 60.0, SR as usize / 2, 0.8),
    ];
    lib.add_file("lib", "kick.wav", wav(SR, &kick));
    lib.add_file(
        "lib",
        "snare.wav",
        wav(SR, &[sine(SR, 200.0, SR as usize / 4, 0.5)]),
    );
    lib.add_file(
        "lib",
        "pad.wav",
        wav(SR, &[sine(SR, 300.0, 3 * SR as usize, 0.3)]),
    );
    lib.add_file("lib", "notes.txt", b"not audio".to_vec());
    lib
}

fn harness() -> H {
    H::new(library(), ControllerConfig::default())
}

fn resampled(frames: usize) -> usize {
    ether_media::resampled_len(frames, SR, ENGINE_SR)
}

#[test]
fn preview_starts_decoded_at_the_engine_rate_and_finishes() {
    let mut h = harness();
    let out = h.preview(lib_source("kick.wav"));
    assert_eq!(ok(&out), ReplyValue::Unit);
    // A short sample is decoded within the command: it starts before the reply.
    assert_eq!(preview_events(&out), vec![started("kick.wav")]);
    assert_eq!(
        h.calls(),
        vec![
            PreviewCall::Stop,
            PreviewCall::Play(1, resampled(SR as usize / 2), 2, ENGINE_SR)
        ]
    );
    // Plays until the engine reports its natural end.
    assert!(preview_events(&h.tick()).is_empty());
    h.ctl.bridge.ended = Some(1);
    assert_eq!(
        preview_events(&h.tick()),
        vec![ended("kick.wav", PreviewEndReason::Finished)]
    );
    // Exactly one PreviewEnded: a duplicate report or a stop afterwards emits nothing.
    h.ctl.bridge.ended = Some(1);
    assert!(preview_events(&h.tick()).is_empty());
    let out = h.stop();
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert!(preview_events(&out).is_empty());
}

#[test]
fn stop_ends_the_preview_and_ignores_its_late_end() {
    let mut h = harness();
    ok(&h.preview(lib_source("snare.wav")));
    h.calls();
    let out = h.stop();
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(
        preview_events(&out),
        vec![ended("snare.wav", PreviewEndReason::Stopped)]
    );
    assert_eq!(h.calls(), vec![PreviewCall::Stop]);
    // The engine's end of the stopped preview (it was already at its end) is ignored.
    h.ctl.bridge.ended = Some(1);
    assert!(preview_events(&h.tick()).is_empty());
    // Stopping with nothing playing is a no-op.
    assert_eq!(ok(&h.stop()), ReplyValue::Unit);
}

#[test]
fn replace_ends_the_old_preview_before_the_new_one_starts() {
    let mut h = harness();
    ok(&h.preview(lib_source("kick.wav")));
    h.calls();
    let out = h.preview(lib_source("snare.wav"));
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(
        preview_events(&out),
        vec![
            ended("kick.wav", PreviewEndReason::Replaced),
            started("snare.wav")
        ]
    );
    assert_eq!(
        h.calls(),
        vec![
            PreviewCall::Stop,
            PreviewCall::Play(2, resampled(SR as usize / 4), 1, ENGINE_SR)
        ]
    );
    h.ctl.bridge.ended = Some(2);
    assert_eq!(
        preview_events(&h.tick()),
        vec![ended("snare.wav", PreviewEndReason::Finished)]
    );
}

/// The race of CONTRACTS.md §11.15: the old preview reaches its natural end in the engine
/// while the controller replaces it, so the poll after the replace reports the *old* id.
/// That must never become `Finished` for the old preview (it already got `Replaced`), nor
/// end the new one.
#[test]
fn replace_in_the_same_poll_as_the_old_natural_end_never_finishes_the_old_id() {
    let mut h = harness();
    ok(&h.preview(lib_source("kick.wav")));
    // The kick (id 1) ends in the engine; before the controller polls, the user previews
    // the snare (id 2).
    h.ctl.bridge.ended = Some(1);
    let out = h.preview(lib_source("snare.wav"));
    assert_eq!(
        preview_events(&out),
        vec![
            ended("kick.wav", PreviewEndReason::Replaced),
            started("snare.wav")
        ]
    );
    // The poll carries id 1: ignored. The snare keeps playing.
    assert!(preview_events(&h.tick()).is_empty());
    assert!(preview_events(&h.tick()).is_empty());
    // Only the snare's own end finishes it.
    h.ctl.bridge.ended = Some(2);
    assert_eq!(
        preview_events(&h.tick()),
        vec![ended("snare.wav", PreviewEndReason::Finished)]
    );

    // Same race while the replacement is still decoding (a long file, a small budget): the
    // old id's end is ignored and nothing finishes before the new preview even started.
    let mut h = H::new(
        library(),
        ControllerConfig {
            media_frames_per_tick: 4_096,
            ..Default::default()
        },
    );
    ok(&h.preview(lib_source("snare.wav")));
    h.drain_until_started();
    h.ctl.bridge.ended = Some(1);
    let out = h.preview(lib_source("pad.wav"));
    assert_eq!(
        preview_events(&out),
        vec![ended("snare.wav", PreviewEndReason::Replaced)]
    );
    let mut evs = preview_events(&h.tick());
    // A stale report of the *new* id can't exist yet; a stale one of the old id arrives.
    h.ctl.bridge.ended = Some(1);
    evs.extend(h.drain_until_started());
    assert_eq!(evs, vec![started("pad.wav")]);
    h.ctl.bridge.ended = Some(2);
    assert_eq!(
        preview_events(&h.tick()),
        vec![ended("pad.wav", PreviewEndReason::Finished)]
    );
}

impl H {
    /// Tick until a `PreviewStarted` (bounded); returns the preview events seen.
    fn drain_until_started(&mut self) -> Vec<Ev> {
        let mut all = Vec::new();
        for _ in 0..1_000 {
            let evs = preview_events(&self.tick());
            let done = evs.iter().any(|e| matches!(e, Ev::Started(_)));
            all.extend(evs);
            if done {
                return all;
            }
        }
        panic!("preview never started: {all:?}");
    }
}

#[test]
fn long_files_decode_over_several_ticks_and_can_be_stopped_meanwhile() {
    let config = ControllerConfig {
        media_frames_per_tick: 8_192,
        ..Default::default()
    };
    let mut h = H::new(library(), config.clone());
    let out = h.preview(lib_source("pad.wav"));
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert!(preview_events(&out).is_empty(), "not decoded yet");
    let mut ticks = 0;
    loop {
        ticks += 1;
        let evs = preview_events(&h.tick());
        if !evs.is_empty() {
            assert_eq!(evs, vec![started("pad.wav")]);
            break;
        }
        assert!(ticks < 1_000);
    }
    // 3 s of 44.1 kHz decode + resample at 8192 frames of work per tick.
    assert!(ticks >= 10, "work is bounded per tick ({ticks} ticks)");
    assert_eq!(
        h.calls().last(),
        Some(&PreviewCall::Play(
            1,
            resampled(3 * SR as usize),
            1,
            ENGINE_SR
        ))
    );

    // Stopping while still decoding ends it at once; it never starts.
    let mut h = H::new(library(), config);
    ok(&h.preview(lib_source("pad.wav")));
    h.tick();
    let out = h.stop();
    assert_eq!(
        preview_events(&out),
        vec![ended("pad.wav", PreviewEndReason::Stopped)]
    );
    for _ in 0..50 {
        assert!(preview_events(&h.tick()).is_empty());
    }
    assert!(!h.calls().iter().any(|c| matches!(c, PreviewCall::Play(..))));
}

#[test]
fn previews_are_capped_in_length() {
    // 40 s at 8 kHz, engine at 8 kHz (no resampling): only the first 30 s are decoded.
    let rate = 8_000;
    let mut lib = MemoryLibrary::new();
    lib.add_file(
        "lib",
        "long.wav",
        wav(rate, &[vec![0.1; 40 * rate as usize]]),
    );
    let mut h = H::new(
        lib,
        ControllerConfig {
            engine_sample_rate: rate,
            ..Default::default()
        },
    );
    ok(&h.preview(lib_source("long.wav")));
    h.drain_until_started();
    // `media_preview::MAX_PREVIEW_SECONDS` = 30.
    let cap = 30 * rate as usize;
    assert_eq!(h.calls().last(), Some(&PreviewCall::Play(1, cap, 1, rate)));
}

#[test]
fn a_repeated_preview_comes_from_the_cache() {
    let mut h = harness();
    ok(&h.preview(lib_source("kick.wav")));
    ok(&h.stop());
    h.calls();
    // The file is gone from the library: the second preview is served from the cache,
    // replacing nothing (it was stopped) and without a separate stop.
    let mut lib = library();
    lib.add_file("lib", "kick.wav", Vec::new());
    h.ctl.library = lib;
    let out = h.preview(lib_source("kick.wav"));
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(preview_events(&out), vec![started("kick.wav")]);
    assert_eq!(
        h.calls(),
        vec![PreviewCall::Play(
            2,
            resampled(SR as usize / 2),
            2,
            ENGINE_SR
        )]
    );
}

#[test]
fn project_media_can_be_previewed() {
    let mut h = harness();
    // Project sources need an open project.
    let out = h.preview(MediaSource::Location {
        location: BrowseLocation::ProjectMedia,
        path: "kick.wav".into(),
    });
    assert_eq!(err(&out).code, ErrorCode::InvalidState);

    h.create_project();
    let media: MediaId = h.ids.next(T0);
    let out = h.send(Command::Media(MediaCommand::Import {
        id: media,
        source: lib_source("snare.wav"),
    }));
    let ReplyValue::Media { media: m } = ok(&out) else {
        panic!("{out:#?}")
    };
    let out = h.preview(MediaSource::Project { media });
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(
        preview_events(&out),
        vec![started(&format!("project:{media}"))]
    );
    // The same file through the project's media folder.
    let path = m
        .file
        .strip_prefix("media/")
        .expect("in media/")
        .to_string();
    let out = h.preview(MediaSource::Location {
        location: BrowseLocation::ProjectMedia,
        path: path.clone(),
    });
    assert_eq!(
        preview_events(&out),
        vec![
            ended(&format!("project:{media}"), PreviewEndReason::Replaced),
            started(&path)
        ]
    );
    let unknown: MediaId = h.ids.next(T0);
    assert_eq!(
        err(&h.preview(MediaSource::Project { media: unknown })).code,
        ErrorCode::NotFound
    );
}

#[test]
fn bad_sources_are_replied_and_leave_the_current_preview_alone() {
    let mut h = harness();
    ok(&h.preview(lib_source("kick.wav")));
    h.calls();
    for (source, code) in [
        (lib_source("missing.wav"), ErrorCode::NotFound),
        (lib_source("notes.txt"), ErrorCode::Decode),
        (lib_source("../kick.wav"), ErrorCode::InvalidArgument),
        (
            MediaSource::Upload {
                upload: "u1".into(),
            },
            ErrorCode::Unsupported,
        ),
    ] {
        let out = h.preview(source.clone());
        assert_eq!(err(&out).code, code, "{source:?}");
        assert!(preview_events(&out).is_empty());
    }
    assert!(h.calls().is_empty(), "the kick keeps playing");
    h.ctl.bridge.ended = Some(1);
    assert_eq!(
        preview_events(&h.tick()),
        vec![ended("kick.wav", PreviewEndReason::Finished)]
    );
}

#[test]
fn an_engine_failure_ends_the_preview_as_failed() {
    let mut h = harness();
    h.ctl.bridge.fail_play = true;
    let out = h.preview(lib_source("kick.wav"));
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert_eq!(
        preview_events(&out),
        vec![ended("kick.wav", PreviewEndReason::Failed)]
    );
    assert!(events(&out).iter().any(|e| matches!(
        e,
        Event::Notification { level: NotificationLevel::Error, message } if message.contains("kick.wav")
    )));
    // Nothing is current afterwards.
    assert!(preview_events(&h.stop()).is_empty());
}

#[test]
fn hosts_without_preview_reply_unsupported() {
    let mut h = Harness::with(FakeBridge::default(), library(), Default::default());
    for c in [
        MediaCommand::Preview {
            source: lib_source("kick.wav"),
        },
        MediaCommand::StopPreview,
    ] {
        let out = h.send(Command::Media(c));
        assert_eq!(err(&out).code, ErrorCode::Unsupported);
        assert!(preview_events(&out).is_empty());
    }
}
