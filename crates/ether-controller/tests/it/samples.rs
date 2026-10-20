//! #519 (docs/SAMPLES.md): the default sample pack. The manifest is fetched and verified on
//! first use, files download lazily into the engine-side cache (size + sha256 checked), the
//! cached pack works offline, newer versions replace the manifest and keep the files projects
//! still reference, and the built-in kit is always there.

use crate::common;

use common::{FakeBridge, Harness, events, ok};
use ether_controller::ControllerConfig;
use ether_controller::memory::MemoryLibrary;
use ether_controller::sample_pack::fixture::{DevPack, dev_pack};
use ether_controller::sample_pack::manifest;
use ether_core::protocol::samples::{PackStatus, SamplesCommand, SamplesEvent, SamplesView};
use ether_core::protocol::*;

const URL: &str = "http://127.0.0.1:4100/packs/default/manifest.json";

fn serve(h: &mut Harness, pack: &DevPack) {
    h.ctl
        .host
        .web
        .insert(URL.into(), (200, pack.envelope.clone()));
    for (path, bytes) in &pack.files {
        h.ctl
            .host
            .web
            .insert(manifest::file_url(URL, path), (200, bytes.clone()));
    }
}

fn open(h: &mut Harness) -> SamplesView {
    match h.ok(Command::Samples(SamplesCommand::Open {
        url: Some(URL.into()),
    })) {
        ReplyValue::Samples { view } => view,
        other => panic!("{other:?}"),
    }
}

fn samples_events(out: &[ServerMessage]) -> Vec<SamplesEvent> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Samples { event } => Some(event),
            _ => None,
        })
        .collect()
}

/// Tick until the fake web answered everything; the samples events.
fn settle(h: &mut Harness) -> Vec<SamplesEvent> {
    let mut all = Vec::new();
    for _ in 0..20 {
        all.extend(samples_events(&h.tick()));
    }
    all
}

fn last_view(events: &[SamplesEvent]) -> SamplesView {
    events
        .iter()
        .rev()
        .find_map(|e| match e {
            SamplesEvent::Changed { view } => Some(view.clone()),
            _ => None,
        })
        .expect("a Changed event")
}

fn fetch(h: &mut Harness, ids: &[&str]) -> Vec<SamplesEvent> {
    let out = h.send(Command::Samples(SamplesCommand::Fetch {
        samples: ids.iter().map(|s| s.to_string()).collect(),
    }));
    assert_eq!(ok(&out), ReplyValue::Unit);
    let mut all = samples_events(&out);
    all.extend(settle(h));
    all
}

fn fetched(events: &[SamplesEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            SamplesEvent::Fetched { sample } => Some(sample.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn without_a_published_pack_only_the_builtin_kit_shows() {
    let mut h = Harness::new();
    let view = open(&mut h);
    assert_eq!(view.status, PackStatus::Loading);
    assert_eq!(view.kits.len(), 1);
    assert_eq!(view.kits[0].id, "builtin/basic");
    assert!(view.samples.iter().all(|s| s.cached));
    let view = last_view(&settle(&mut h));
    assert_eq!(view.status, PackStatus::ComingSoon);
    assert!(view.pack.is_none());
    // The built-in samples "download" at once and read from code.
    let ev = fetch(&mut h, &["builtin/kick", "builtin/hihat"]);
    assert_eq!(fetched(&ev), ["builtin/kick", "builtin/hihat"]);
}

#[test]
fn files_download_lazily_verified_and_once() {
    let mut h = Harness::new();
    let pack = dev_pack(1, 1.0);
    serve(&mut h, &pack);
    open(&mut h);
    let view = last_view(&settle(&mut h));
    assert_eq!(view.status, PackStatus::Ready { offline: false });
    assert_eq!(view.pack.as_ref().unwrap().version, 1);
    assert_eq!(view.kits.len(), 2);
    assert_eq!(view.instruments[0].zones[0].sample, "default/keys/sine-c4");
    assert!(
        view.samples
            .iter()
            .filter(|s| !s.id.starts_with("builtin/"))
            .all(|s| !s.cached)
    );
    // Only the manifest was fetched: files wait for first use.
    assert_eq!(h.ctl.host.requested, [URL]);

    let ev = fetch(&mut h, &["default/drums/a/kick", "default/drums/a/snare"]);
    assert_eq!(fetched(&ev).len(), 2);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SamplesEvent::Progress { .. }))
    );
    let Some(SamplesEvent::Progress {
        done,
        total,
        pending,
    }) = ev
        .iter()
        .rev()
        .find(|e| matches!(e, SamplesEvent::Progress { .. }))
        .cloned()
    else {
        unreachable!()
    };
    assert_eq!((done, pending), (total, 0));
    assert_eq!(h.ctl.host.requested.len(), 3);
    // Cached now: a second fetch answers at once, no request.
    let ev = fetch(&mut h, &["default/drums/a/kick"]);
    assert_eq!(fetched(&ev), ["default/drums/a/kick"]);
    assert_eq!(h.ctl.host.requested.len(), 3);
    let view = match h.ok(Command::Samples(SamplesCommand::Get)) {
        ReplyValue::Samples { view } => view,
        other => panic!("{other:?}"),
    };
    let cached: Vec<_> = view
        .samples
        .iter()
        .filter(|s| s.cached && !s.id.starts_with("builtin/"))
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(cached, ["default/drums/a/kick", "default/drums/a/snare"]);
}

#[test]
fn a_damaged_download_is_refused() {
    let mut h = Harness::new();
    let pack = dev_pack(1, 1.0);
    serve(&mut h, &pack);
    let kick = &pack.manifest.files[0];
    let mut evil = pack.files[0].1.clone();
    let n = evil.len();
    evil[n - 1] ^= 0xff;
    h.ctl
        .host
        .web
        .insert(manifest::file_url(URL, &kick.path), (200, evil));
    open(&mut h);
    settle(&mut h);
    let ev = fetch(&mut h, &["default/drums/a/kick"]);
    assert!(fetched(&ev).is_empty());
    assert!(ev.iter().any(|e| matches!(e,
        SamplesEvent::Failed { sample, reason }
            if sample == "default/drums/a/kick" && reason.contains("checksum"))));
    assert!(
        h.ctl
            .library
            .sample_cache()
            .iter()
            .all(|f| !f.contains("/files/"))
    );
}

#[test]
fn a_manifest_signed_with_another_key_is_refused() {
    let mut h = Harness::new();
    let pack = dev_pack(1, 1.0);
    serve(&mut h, &pack);
    // The same pack from a non-loopback source: the dev key isn't accepted there.
    let remote = "https://samples.example.com/manifest.json";
    h.ctl
        .host
        .web
        .insert(remote.into(), (200, pack.envelope.clone()));
    h.ok(Command::Samples(SamplesCommand::Open {
        url: Some(remote.into()),
    }));
    let view = last_view(&settle(&mut h));
    assert!(
        matches!(view.status, PackStatus::Invalid { .. }),
        "{view:?}"
    );
    assert_eq!(view.kits.len(), 1, "the built-in kit still shows");
}

#[test]
fn the_cached_pack_works_offline_after_a_restart() {
    let mut h = Harness::new();
    let pack = dev_pack(1, 1.0);
    serve(&mut h, &pack);
    open(&mut h);
    settle(&mut h);
    fetch(&mut h, &["default/drums/a/kick"]);
    let library = h.ctl.library.clone();

    // A new session without network, on the same machine.
    let mut h = Harness::with(FakeBridge::default(), library, ControllerConfig::default());
    h.ctl.host.offline = true;
    let view = open(&mut h);
    assert_eq!(
        view.pack.as_ref().unwrap().version,
        1,
        "the cached manifest at once"
    );
    let view = last_view(&settle(&mut h));
    assert_eq!(view.status, PackStatus::Ready { offline: true });
    let kick = view
        .samples
        .iter()
        .find(|s| s.id == "default/drums/a/kick")
        .unwrap();
    assert!(kick.cached);
    let ev = fetch(&mut h, &["default/drums/a/kick", "default/drums/a/snare"]);
    assert_eq!(fetched(&ev), ["default/drums/a/kick"]);
    assert!(ev.iter().any(|e| matches!(e,
        SamplesEvent::Failed { sample, reason }
            if sample == "default/drums/a/snare" && reason == "offline")));

    // Back online: opening again retries the refresh.
    h.ctl.host.offline = false;
    serve(&mut h, &pack);
    open(&mut h);
    let view = last_view(&settle(&mut h));
    assert_eq!(view.status, PackStatus::Ready { offline: false });
}

#[test]
fn offline_with_nothing_cached_shows_the_builtin_kit() {
    let mut h = Harness::new();
    h.ctl.host.offline = true;
    open(&mut h);
    let view = last_view(&settle(&mut h));
    assert_eq!(view.status, PackStatus::Offline);
    assert_eq!(view.kits.len(), 1);
}

#[test]
fn a_host_without_cache_or_network_still_has_the_builtin_kit() {
    let mut h = Harness::with(
        FakeBridge::default(),
        MemoryLibrary::new().without_sample_cache(),
        ControllerConfig::default(),
    );
    h.ok(Command::Samples(SamplesCommand::Open {
        url: Some(String::new()),
    }));
    let view = match h.ok(Command::Samples(SamplesCommand::Get)) {
        ReplyValue::Samples { view } => view,
        other => panic!("{other:?}"),
    };
    assert_eq!(view.status, PackStatus::ComingSoon);
    assert!(h.ctl.host.requested.is_empty());
    let ev = fetch(&mut h, &["builtin/snare"]);
    assert_eq!(fetched(&ev), ["builtin/snare"]);
}

#[test]
fn a_newer_version_replaces_the_pack_and_an_older_one_is_refused() {
    let mut h = Harness::new();
    let v1 = dev_pack(1, 1.0);
    serve(&mut h, &v1);
    open(&mut h);
    settle(&mut h);
    fetch(&mut h, &["default/keys/sine-c4", "default/drums/a/kick"]);
    let old_keys = v1.manifest.files[4].sha256.clone();

    // v2 changes the instrument file. A restart picks it up.
    let library = h.ctl.library.clone();
    let mut h = Harness::with(FakeBridge::default(), library, ControllerConfig::default());
    let v2 = dev_pack(2, 2.0);
    serve(&mut h, &v2);
    open(&mut h);
    let view = last_view(&settle(&mut h));
    assert_eq!(view.pack.as_ref().unwrap().version, 2);
    let keys = view
        .samples
        .iter()
        .find(|s| s.id == "default/keys/sine-c4")
        .unwrap();
    assert!(!keys.cached, "the new file isn't downloaded yet");
    let kick = view
        .samples
        .iter()
        .find(|s| s.id == "default/drums/a/kick")
        .unwrap();
    assert!(kick.cached, "unchanged files stay");
    let cache = h.ctl.library.sample_cache();
    assert!(
        !cache.iter().any(|f| f.contains(&old_keys)),
        "the replaced, unreferenced file was collected"
    );

    // Serving v1 again (a rollback) keeps v2.
    let library = h.ctl.library.clone();
    let mut h = Harness::with(FakeBridge::default(), library, ControllerConfig::default());
    serve(&mut h, &v1);
    let view = open(&mut h);
    assert_eq!(view.pack.as_ref().unwrap().version, 2);
    let ev = settle(&mut h);
    assert!(
        ev.iter()
            .all(|e| !matches!(e, SamplesEvent::Changed { .. }))
    );
}

#[test]
fn http_sources_other_than_loopback_are_refused() {
    let mut h = Harness::new();
    h.ok(Command::Samples(SamplesCommand::Open {
        url: Some("http://samples.example.com/manifest.json".into()),
    }));
    let view = match h.ok(Command::Samples(SamplesCommand::Get)) {
        ReplyValue::Samples { view } => view,
        other => panic!("{other:?}"),
    };
    assert!(matches!(view.status, PackStatus::Invalid { .. }));
    assert!(h.ctl.host.requested.is_empty());
}
