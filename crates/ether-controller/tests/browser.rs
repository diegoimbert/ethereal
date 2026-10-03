//! Browser v2 (`browser-v2`; CONTRACTS.md §12.8): the library index through the controller
//! with an in-memory library: background indexing, search and filters, packs, presets,
//! projects, favourites/tags, persistence and reload, incremental rescans, errors, and the
//! tempo-synced preview (repitch ratio + next-beat start) with a recording bridge.

mod common;

use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::Library;
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::protocol::browser::*;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::media::{BrowseLocation, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::presets::{PresetRef, PresetSource};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;
use ether_core::transport::PlayheadState;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

const LIB: &str = "lib";
const USER: &str = "user";
const SR: u32 = 48_000;

/// `FakeBridge` plus preview support: records `(frames, sample rate)` of each play.
#[derive(Default)]
struct Bridge {
    inner: FakeBridge,
    plays: Vec<(usize, u32)>,
}

impl EngineBridge for Bridge {
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
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn preview(
        &mut self,
        _id: u64,
        audio: Option<Arc<DecodedAudio>>,
        _gain: f32,
    ) -> Result<(), BridgeError> {
        if let Some(a) = audio {
            self.plays.push((a.frames(), a.sample_rate));
        }
        Ok(())
    }
}

struct H {
    ctl: EtherController<Bridge, FakeHost, MemoryStore, MemoryLibrary>,
    ids: IdGen,
    next: u32,
}

impl H {
    fn new(lib: MemoryLibrary) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        Self {
            ctl: EtherController::with_config(
                Bridge::default(),
                FakeHost { now: T0 },
                store,
                lib,
                ControllerConfig::default(),
            ),
            ids: IdGen::new(5),
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

    fn ok(&mut self, c: BrowserCommand) -> ReplyValue {
        ok(&self.send(Command::Browser(c)))
    }

    fn err(&mut self, c: BrowserCommand) -> CommandError {
        err(&self.send(Command::Browser(c)))
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    fn advance(&mut self, ms: u64) {
        self.ctl.host.now += ms;
        self.ctl.store.now_ms = self.ctl.host.now;
    }

    /// Tick until indexing (listing and probing) is idle; returns the browser events.
    fn index(&mut self) -> Vec<BrowserEvent> {
        let mut all = Vec::new();
        let mut quiet = 0;
        for _ in 0..10_000 {
            self.advance(20);
            let evs: Vec<BrowserEvent> = events(&self.tick())
                .into_iter()
                .filter_map(|e| match e {
                    Event::Browser { event } => Some(event),
                    _ => None,
                })
                .collect();
            quiet = if evs.is_empty() { quiet + 1 } else { 0 };
            all.extend(evs);
            if quiet > 150 {
                return all;
            }
        }
        panic!("indexing never settled");
    }

    fn query(&mut self, q: BrowserQuery) -> BrowserPage {
        match self.ok(BrowserCommand::Query { query: q }) {
            ReplyValue::BrowserPage { page } => page,
            other => panic!("{other:?}"),
        }
    }

    fn roots(&mut self) -> Vec<BrowserRoot> {
        match self.ok(BrowserCommand::ListRoots) {
            ReplyValue::BrowserRoots { roots } => roots,
            other => panic!("{other:?}"),
        }
    }

    fn create_project(&mut self, name: &str) {
        let id = self.ids.next_project_id(T0);
        ok(&self.send(Command::Project(ProjectCommand::Create {
            id,
            name: name.into(),
        })));
    }
}

fn q(text: &str) -> BrowserQuery {
    BrowserQuery {
        text: text.into(),
        kinds: vec![],
        tags: vec![],
        favourites_only: false,
        roots: vec![],
        folder: None,
        device: None,
        sort: BrowserSort::Name,
        offset: 0,
        limit: 200,
    }
}

fn ids(p: &BrowserPage) -> Vec<&str> {
    p.items.iter().map(|i| i.id.as_str()).collect()
}

fn tone(seconds: f64) -> Vec<u8> {
    wav(
        44_100,
        &[sine(44_100, 220.0, (seconds * 44_100.0) as usize, 0.5)],
    )
}

fn user_preset(name: &str, tags: &[&str]) -> Vec<u8> {
    let f = &ether_devices::factory_presets(BuiltinDeviceType::Synth)[0];
    let mut p = load_preset(f.json).unwrap();
    p.name = name.into();
    p.meta.tags = tags.iter().map(|t| t.to_string()).collect();
    save_preset(&p, "test").unwrap().into_bytes()
}

fn library() -> MemoryLibrary {
    let mut lib = MemoryLibrary::new().with_user_root(USER);
    lib.add_root(LIB, "Samples");
    lib.add_file(LIB, "Drums/Kick.wav", tone(0.5));
    lib.add_file(LIB, "Drums/Snare 100.wav", tone(0.25));
    lib.add_file(LIB, "Drums/Loops/Break 120.wav", tone(2.0));
    lib.add_file(LIB, "Bass/Bass_Am_128bpm.wav", tone(1.0));
    lib.add_file(LIB, "Bass/notes.txt", b"x".to_vec());
    lib.add_file(LIB, "Midi/Groove.mid", b"MThd".to_vec());
    lib.add_file(LIB, "Vinyl/pack.json", br#"{"name":"Vinyl Kit"}"#.to_vec());
    lib.add_file(LIB, "Vinyl/Crackle Kick.wav", tone(0.1));
    lib.add_file(LIB, ".hidden/secret.wav", tone(0.1));
    lib.write_file(
        USER,
        "Presets/synth/Warm Pad.etherpreset",
        &user_preset("Warm Pad", &["Pad", "warm"]),
    )
    .unwrap();
    lib.write_file(USER, "Samples/Vox.wav", &tone(0.3)).unwrap();
    lib
}

fn indexed() -> H {
    let mut h = H::new(library());
    h.roots();
    h.index();
    h
}

#[test]
fn indexes_library_presets_and_projects_in_the_background() {
    let mut h = H::new(library());
    h.create_project("Song A");
    // Nothing is scanned before the first browser command.
    assert!(
        events(&h.tick())
            .iter()
            .all(|e| !matches!(e, Event::Browser { .. }))
    );
    let roots = h.roots();
    let evs = h.index();
    assert!(evs.iter().any(|e| matches!(e, BrowserEvent::IndexProgress { root, total: Some(n), .. } if root == LIB && *n == 6)));
    assert!(evs.contains(&BrowserEvent::IndexChanged));
    let names: Vec<(&str, BrowserRootKind)> =
        roots.iter().map(|r| (r.id.as_str(), r.kind)).collect();
    assert_eq!(
        names,
        [
            (LIB, BrowserRootKind::Library),
            (USER, BrowserRootKind::Library),
            ("factory", BrowserRootKind::Factory)
        ]
    );

    let roots = h.roots();
    let pack = roots
        .iter()
        .find(|r| r.id == "lib/Vinyl")
        .expect("pack root");
    assert_eq!(
        (pack.name.as_str(), pack.kind, pack.items),
        ("Vinyl Kit", BrowserRootKind::Pack, 1)
    );
    assert_eq!(roots.iter().find(|r| r.id == LIB).unwrap().items, 6);

    let all = h.query(BrowserQuery {
        kinds: vec![LibraryItemKind::Audio, LibraryItemKind::Midi],
        ..q("")
    });
    assert_eq!(
        ids(&all),
        [
            "lib/Bass/Bass_Am_128bpm.wav",
            "lib/Drums/Loops/Break 120.wav",
            "lib/Vinyl/Crackle Kick.wav",
            "lib/Midi/Groove.mid",
            "lib/Drums/Kick.wav",
            "lib/Drums/Snare 100.wav",
            "user/Samples/Vox.wav",
        ]
    );
    let bass = &all.items[0];
    assert_eq!(bass.meta.bpm, Some(128.0));
    assert_eq!(bass.meta.key.as_deref(), Some("A minor"));
    assert_eq!(bass.meta.pack.as_deref(), Some("Samples"));
    assert_eq!(bass.meta.sample_rate, Some(44_100));
    assert_eq!(bass.meta.channels, Some(1));
    assert!((bass.meta.duration_seconds.unwrap() - 1.0).abs() < 1e-3);
    assert_eq!(
        bass.source,
        Some(MediaSource::Location {
            location: BrowseLocation::Library { id: LIB.into() },
            path: "Bass/Bass_Am_128bpm.wav".into(),
        })
    );
    assert_eq!(all.items[1].meta.bpm, Some(120.0), "a loop's bare number");
    assert_eq!(
        all.items[5].meta.bpm, None,
        "a bare number without loop context"
    );
    assert_eq!(all.items[2].meta.pack.as_deref(), Some("Vinyl Kit"));

    // User preset: tags from its meta, device filter.
    let pads = h.query(BrowserQuery {
        kinds: vec![LibraryItemKind::Preset],
        roots: vec![USER.into()],
        device: Some(PresetDevice::Builtin {
            device: BuiltinDeviceType::Synth,
        }),
        ..q("warm")
    });
    assert_eq!(ids(&pads), ["user/Presets/synth/Warm Pad.etherpreset"]);
    assert_eq!(pads.items[0].tags, ["pad", "warm"]);
    assert_eq!(
        pads.items[0].preset,
        Some(PresetRef {
            source: PresetSource::User,
            id: "synth/Warm Pad.etherpreset".into(),
        })
    );
    let factory = h.query(BrowserQuery {
        roots: vec!["factory".into()],
        ..q("")
    });
    assert!(factory.total > 0);
    assert!(
        factory
            .items
            .iter()
            .all(|i| i.kind == LibraryItemKind::Preset
                && i.preset.as_ref().unwrap().source == PresetSource::Factory)
    );
    let no_delay_presets = h.query(BrowserQuery {
        kinds: vec![LibraryItemKind::Preset],
        roots: vec![USER.into()],
        device: Some(PresetDevice::Builtin {
            device: BuiltinDeviceType::Delay,
        }),
        ..q("warm")
    });
    assert_eq!(no_delay_presets.total, 0);

    let projects = h.query(BrowserQuery {
        kinds: vec![LibraryItemKind::Project],
        ..q("song")
    });
    assert_eq!(projects.total, 1);
    assert_eq!(projects.items[0].name, "Song A");
    assert!(projects.items[0].meta.modified_ms.is_some());
}

#[test]
fn search_filters_sort_and_paging() {
    let mut h = indexed();
    assert_eq!(
        ids(&h.query(q("kick"))),
        ["lib/Vinyl/Crackle Kick.wav", "lib/Drums/Kick.wav"]
    );
    assert_eq!(
        ids(&h.query(BrowserQuery {
            sort: BrowserSort::Relevance,
            ..q("kick")
        })),
        ["lib/Drums/Kick.wav", "lib/Vinyl/Crackle Kick.wav"]
    );
    // Words match name, path, pack and key.
    assert_eq!(
        ids(&h.query(q("drums loops"))),
        ["lib/Drums/Loops/Break 120.wav"]
    );
    assert_eq!(
        ids(&h.query(q("vinyl kit"))),
        ["lib/Vinyl/Crackle Kick.wav"]
    );
    // Scoped to the sample library: factory presets (e.g. the scale-quantize "C minor
    // pentatonic") also match "minor".
    assert_eq!(
        ids(&h.query(BrowserQuery {
            roots: vec!["lib".into()],
            ..q("a minor")
        })),
        ["lib/Bass/Bass_Am_128bpm.wav"]
    );
    // Roots, packs, folders.
    assert_eq!(
        ids(&h.query(BrowserQuery {
            roots: vec!["lib/Vinyl".into()],
            ..q("")
        })),
        ["lib/Vinyl/Crackle Kick.wav"]
    );
    let drums = h.query(BrowserQuery {
        roots: vec![LIB.into()],
        folder: Some("Drums".into()),
        ..q("")
    });
    assert_eq!(drums.total, 3);
    assert_eq!(
        h.query(BrowserQuery {
            roots: vec![USER.into()],
            kinds: vec![LibraryItemKind::Audio],
            ..q("")
        })
        .total,
        1
    );
    // Sorts.
    let by_bpm = h.query(BrowserQuery {
        sort: BrowserSort::Bpm,
        kinds: vec![LibraryItemKind::Audio],
        ..q("")
    });
    assert_eq!(
        &ids(&by_bpm)[..2],
        [
            "lib/Drums/Loops/Break 120.wav",
            "lib/Bass/Bass_Am_128bpm.wav"
        ]
    );
    let by_duration = h.query(BrowserQuery {
        sort: BrowserSort::Duration,
        kinds: vec![LibraryItemKind::Audio],
        ..q("")
    });
    assert_eq!(
        by_duration.items.last().unwrap().id,
        "lib/Drums/Loops/Break 120.wav"
    );
    // Paging (limit clamped to 1..=200).
    let page = h.query(BrowserQuery {
        offset: 2,
        limit: 2,
        kinds: vec![LibraryItemKind::Audio],
        ..q("")
    });
    assert_eq!((page.total, page.offset, page.items.len()), (6, 2, 2));
    let one = h.query(BrowserQuery { limit: 0, ..q("") });
    assert_eq!(one.items.len(), 1);
    let big = h.query(BrowserQuery {
        limit: 10_000,
        ..q("")
    });
    assert!(big.items.len() <= 200);
}

#[test]
fn favourites_and_tags_persist_and_reload() {
    let mut h = indexed();
    let kick = "lib/Drums/Kick.wav";
    let out = h.send(Command::Browser(BrowserCommand::SetFavourite {
        item: kick.into(),
        favourite: true,
    }));
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert!(events(&out).contains(&Event::Browser {
        event: BrowserEvent::IndexChanged
    }));
    h.ok(BrowserCommand::SetTags {
        item: kick.into(),
        tags: vec!["Punchy".into(), " dry ".into(), "punchy".into(), "".into()],
    });
    let fav = h.query(BrowserQuery {
        favourites_only: true,
        ..q("")
    });
    assert_eq!(ids(&fav), [kick]);
    assert_eq!(fav.items[0].tags, ["dry", "punchy"]);
    assert!(fav.items[0].favourite);
    // Tag filter and text over tags.
    assert_eq!(
        ids(&h.query(BrowserQuery {
            tags: vec!["DRY".into()],
            ..q("")
        })),
        [kick]
    );
    assert_eq!(
        ids(&h.query(BrowserQuery {
            roots: vec![LIB.into()],
            ..q("punch")
        })),
        [kick]
    );

    // Persisted after a quiet period.
    h.advance(3000);
    h.tick();
    assert!(
        h.ctl
            .library
            .files(USER)
            .contains(&".ethereal/index.json".to_string())
    );

    // A fresh controller over the same library: favourites, tags and probed metadata are
    // back right away (before the rescan finishes), and stay after it.
    let lib = std::mem::take(&mut h.ctl.library);
    let mut h = H::new(lib);
    let fav = h.query(BrowserQuery {
        favourites_only: true,
        ..q("")
    });
    assert_eq!(ids(&fav), [kick]);
    assert_eq!(fav.items[0].tags, ["dry", "punchy"]);
    assert!(
        fav.items[0].meta.duration_seconds.is_some(),
        "probe persisted"
    );
    h.index();
    let fav = h.query(BrowserQuery {
        favourites_only: true,
        ..q("")
    });
    assert_eq!(ids(&fav), [kick]);
    h.ok(BrowserCommand::SetFavourite {
        item: kick.into(),
        favourite: false,
    });
    assert_eq!(
        h.query(BrowserQuery {
            favourites_only: true,
            ..q("")
        })
        .total,
        0
    );
}

#[test]
fn rescan_is_incremental() {
    let mut h = indexed();
    h.ctl.library.add_file(LIB, "Drums/Clap.wav", tone(0.2));
    h.ctl
        .library
        .write_file(USER, "Samples/New.wav", &tone(0.2))
        .unwrap();
    h.ctl.library.remove_file(USER, "Samples/Vox.wav").unwrap();
    // Not seen until a rescan.
    assert_eq!(h.query(q("clap")).total, 0);
    assert_eq!(
        h.ok(BrowserCommand::Rescan {
            root: Some(LIB.into())
        }),
        ReplyValue::Unit
    );
    h.index();
    assert_eq!(ids(&h.query(q("clap"))), ["lib/Drums/Clap.wav"]);
    assert_eq!(
        h.query(q("vox")).total,
        1,
        "the user root was not rescanned"
    );
    h.ok(BrowserCommand::Rescan { root: None });
    h.index();
    assert_eq!(h.query(q("vox")).total, 0);
    assert_eq!(ids(&h.query(q("new"))), ["user/Samples/New.wav"]);
    // A pack id rescans its root; factory/projects are valid roots.
    h.ok(BrowserCommand::Rescan {
        root: Some("lib/Vinyl".into()),
    });
    h.ok(BrowserCommand::Rescan {
        root: Some("factory".into()),
    });
    h.ok(BrowserCommand::Rescan {
        root: Some("projects".into()),
    });
    h.index();
}

#[test]
fn errors() {
    let mut h = indexed();
    assert_eq!(
        h.err(BrowserCommand::SetFavourite {
            item: "lib/nope.wav".into(),
            favourite: true
        })
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err(BrowserCommand::SetTags {
            item: "lib/nope.wav".into(),
            tags: vec![]
        })
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err(BrowserCommand::Rescan {
            root: Some("nope".into())
        })
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err(BrowserCommand::RemoveFolder { root: LIB.into() })
            .code,
        ErrorCode::NotFound
    );
    // The memory library has no OS folders (web/remote behave the same).
    assert_eq!(
        h.err(BrowserCommand::AddFolder {
            path: "/tmp".into()
        })
        .code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        h.err(BrowserCommand::AddFolder { path: " ".into() }).code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        h.err(BrowserCommand::Preview {
            item: "lib/Midi/Groove.mid".into(),
            sync: false
        })
        .code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        h.err(BrowserCommand::Preview {
            item: "lib/none.wav".into(),
            sync: false
        })
        .code,
        ErrorCode::NotFound
    );
}

fn loop_library() -> MemoryLibrary {
    let mut lib = MemoryLibrary::new();
    lib.add_root(LIB, "Samples");
    // One second at the engine rate (no resample unless synced).
    let one_second = wav(SR, &[sine(SR, 110.0, SR as usize, 0.5)]);
    lib.add_file(LIB, "Loops/Beat 60bpm.wav", one_second.clone());
    lib.add_file(LIB, "Loops/Beat 120bpm.wav", one_second.clone());
    lib.add_file(LIB, "Hits/Hit.wav", one_second);
    lib
}

fn preview(h: &mut H, item: &str, sync: bool) -> (usize, u32) {
    h.ctl.bridge.plays.clear();
    let out = h.send(Command::Browser(BrowserCommand::Preview {
        item: item.into(),
        sync,
    }));
    assert_eq!(ok(&out), ReplyValue::Unit);
    for _ in 0..1000 {
        if let Some(p) = h.ctl.bridge.plays.first() {
            return *p;
        }
        h.tick();
    }
    panic!("preview never started");
}

#[test]
fn tempo_synced_preview_repitches_and_waits_for_the_beat() {
    let mut h = H::new(loop_library());
    h.create_project("Sync");
    h.roots();
    h.index();
    let project_bpm = h.ctl.project().unwrap().tempo_map().bpm_at(Beats(0.0));
    assert_eq!(project_bpm, 120.0);
    let close = |(frames, rate): (usize, u32), want: usize| {
        assert_eq!(rate, SR, "labelled with the engine rate");
        assert!(frames.abs_diff(want) <= 64, "{frames} vs {want}");
    };
    // Unsynced: as is.
    close(
        preview(&mut h, "lib/Loops/Beat 60bpm.wav", false),
        SR as usize,
    );
    // 60 bpm item in a 120 bpm project: twice as fast, half as long.
    close(
        preview(&mut h, "lib/Loops/Beat 60bpm.wav", true),
        SR as usize / 2,
    );
    // Same tempo, or no tempo: unchanged.
    close(
        preview(&mut h, "lib/Loops/Beat 120bpm.wav", true),
        SR as usize,
    );
    close(preview(&mut h, "lib/Hits/Hit.wav", true), SR as usize);

    // Playing at beat 4.5: a synced preview waits half a beat (0.25 s at 120 bpm).
    h.ctl.bridge.inner.playhead = Some(PlayheadState {
        playing: true,
        recording: false,
        position: Beats(4.5),
        seconds: 2.25,
        bpm: 120.0,
        sample_time: 0,
    });
    h.send(Command::Transport(
        ether_core::protocol::transport::TransportCommand::Play,
    ));
    h.tick();
    close(
        preview(&mut h, "lib/Loops/Beat 120bpm.wav", true),
        SR as usize + SR as usize / 4,
    );
    // Cached decode, still aligned at hand-off.
    close(
        preview(&mut h, "lib/Loops/Beat 120bpm.wav", true),
        SR as usize + SR as usize / 4,
    );
    // Unsynced previews never wait.
    close(
        preview(&mut h, "lib/Loops/Beat 120bpm.wav", false),
        SR as usize,
    );
}
