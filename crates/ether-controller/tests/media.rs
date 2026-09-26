//! Media import pipeline with a generated WAV: copy into the project, header probe for the
//! reply, async decode → peaks (source rate) → resample (engine rate) → engine; browser.

mod common;

use common::*;
use ether_controller::content_hash;
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::ClipContentDesc;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 44_100;
const FRAMES: usize = 110_250; // 2.5 s

fn library() -> (MemoryLibrary, Vec<u8>) {
    let left = sine(SR, 440.0, FRAMES, 0.5);
    let right = sine(SR, 220.0, FRAMES, 0.25);
    let bytes = wav(SR, &[left, right]);
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file("lib", "Drums/kick.wav", bytes.clone());
    lib.add_file("lib", "notes.txt", b"not audio".to_vec());
    (lib, bytes)
}

fn harness() -> (Harness, Vec<u8>) {
    let (lib, bytes) = library();
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Media");
    (h, bytes)
}

fn import(h: &mut Harness, path: &str) -> (MediaId, Vec<ServerMessage>) {
    let id: MediaId = h.id();
    let out = h.send(Command::Media(MediaCommand::Import {
        id,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: path.into(),
        },
    }));
    (id, out)
}

#[test]
fn import_pipeline() {
    let (mut h, bytes) = harness();
    let (id, out) = import(&mut h, "Drums/kick.wav");
    let ReplyValue::Media { media } = ok(&out) else {
        panic!()
    };
    assert_eq!(media.id, id);
    assert_eq!(
        (media.sample_rate, media.channels, media.frames),
        (SR, 2, FRAMES as u64)
    );
    assert_eq!(media.name, "kick.wav");
    assert_eq!(media.file, format!("media/{id}-kick.wav"));
    assert_eq!(media.hash.as_deref(), Some(content_hash(&bytes).as_str()));
    // Patch (media upsert) before the reply; the copy is in the project folder.
    let p = patches(&out);
    assert!(
        matches!(&p[0].changes[0], PatchChange::Upsert { entity: Entity::Media(m) } if m.id == id)
    );
    let pid = h.project().id;
    assert_eq!(h.ctl.store.file(pid, &media.file), Some(bytes.as_slice()));

    // Nothing decoded on the hot path yet.
    assert!(h.ctl.bridge.media.is_empty());
    let request = PeakRequest {
        media: id,
        samples_per_peak: 1_000,
        start_frame: 0.0,
        frame_count: FRAMES as f64,
    };
    let out = h.send(Command::Media(MediaCommand::GetPeaks {
        request: request.clone(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);

    // Stepped from tick(): several ticks, with progress.
    let first = h.tick();
    assert!(h.ctl.media_pending(), "work is split across ticks");
    assert!(events(&first).iter().any(|e| matches!(e, Event::Media { event: MediaEvent::ImportProgress { media, .. } } if *media == id)));
    let rest = h.drain_media();
    let evs: Vec<Event> = events(&first).into_iter().chain(events(&rest)).collect();
    assert!(evs.contains(&Event::Media {
        event: MediaEvent::PeaksReady { media: id }
    }));
    assert!(evs.contains(&Event::Media {
        event: MediaEvent::ImportProgress {
            media: id,
            progress: 1.0
        }
    }));

    // Engine got it at the engine rate (48 kHz), exact length.
    let audio = &h.ctl.bridge.media[&id];
    assert_eq!(audio.sample_rate, 48_000);
    assert_eq!(audio.channels.len(), 2);
    assert_eq!(
        audio.frames(),
        ether_media::resampled_len(FRAMES, SR, 48_000)
    );
    // Same as the one-shot resampler.
    let reference =
        ether_media::resample(&ether_media::decode(&bytes, Some("wav")).unwrap(), 48_000).unwrap();
    assert_eq!(**audio, reference);

    // Peaks are in *source* frames.
    let ReplyValue::Peaks { peaks } = h.ok(Command::Media(MediaCommand::GetPeaks { request }))
    else {
        panic!()
    };
    assert!(peaks.samples_per_peak >= 1_000);
    assert_eq!(peaks.min.len(), 2);
    assert_eq!(
        peaks.max[0].len(),
        FRAMES.div_ceil(peaks.samples_per_peak as usize)
    );
    let top = peaks.max[0].iter().cloned().fold(0.0f32, f32::max);
    assert!((top - 0.5).abs() < 0.01, "peak {top}");
    // Cached by hash.
    assert!(
        h.ctl
            .store
            .file(pid, &format!("cache/{}.peaks", media.hash.clone().unwrap()))
            .is_some()
    );

    // An audio clip gets its length from the media and the tempo.
    let t: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let c: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: c,
        track: t,
        start: Beats(4.0),
        media: id,
    }));
    assert!(h.project().clips[&c].length.approx_eq(Beats(5.0)));
    assert_eq!(h.project().clips[&c].name, "kick");
    h.advance(100);
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let cd = &g.tracks.iter().find(|x| x.id == t).unwrap().clips[0];
    match &cd.content {
        ClipContentDesc::Audio {
            media, warp, gain, ..
        } => {
            assert_eq!(*media, id);
            assert_eq!(*gain, 1.0);
            let w = warp
                .as_ref()
                .expect("warped at the tempo it was imported at");
            assert_eq!(w.markers, vec![(0.0, 0.0), (1.0, 0.5)]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn reopen_reloads_media_with_cached_peaks() {
    let (mut h, _) = harness();
    let (id, _) = import(&mut h, "Drums/kick.wav");
    h.drain_media();
    h.ok(Command::Project(ProjectCommand::Save));
    let pid = h.project().id;
    h.create_project("Other");
    assert!(
        h.ctl.bridge.media.is_empty(),
        "media of the closed project is unloaded"
    );
    let out = h.send(Command::Project(ProjectCommand::Open { id: pid }));
    ok(&out);
    let evs = events(&h.tick());
    // Peaks come from the cache before any decoding.
    assert!(evs.contains(&Event::Media {
        event: MediaEvent::PeaksReady { media: id }
    }));
    h.drain_media();
    assert_eq!(h.ctl.bridge.media[&id].sample_rate, 48_000);

    // Engine rate change: everything is resampled again.
    h.ctl.set_engine_sample_rate(44_100);
    h.drain_media();
    assert_eq!(h.ctl.bridge.media[&id].sample_rate, 44_100);
    assert_eq!(h.ctl.bridge.media[&id].frames(), FRAMES);
}

#[test]
fn import_is_idempotent_dedupes_and_undoes() {
    let (mut h, _) = harness();
    let (a, out) = import(&mut h, "Drums/kick.wav");
    let ReplyValue::Media { media: ma } = ok(&out) else {
        panic!()
    };
    // Retried message: same media, no new patch.
    let out = h.send(Command::Media(MediaCommand::Import {
        id: a,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "Drums/kick.wav".into(),
        },
    }));
    assert!(patches(&out).is_empty());
    // Same content under a new id: shares the file.
    let (b, out) = import(&mut h, "Drums/kick.wav");
    let ReplyValue::Media { media: mb } = ok(&out) else {
        panic!()
    };
    assert_ne!(a, b);
    assert_eq!(ma.file, mb.file);
    // From the project's own media folder: no copy.
    let rel = ma.file.strip_prefix("media/").unwrap().to_string();
    let c: MediaId = h.id();
    let v = h.ok(Command::Media(MediaCommand::Import {
        id: c,
        source: MediaSource::Location {
            location: BrowseLocation::ProjectMedia,
            path: rel,
        },
    }));
    assert!(matches!(v, ReplyValue::Media { media } if media.file == ma.file));
    h.drain_media();
    assert_eq!(h.ctl.bridge.media.len(), 3);
    // Undo the last import: the engine source goes away.
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(!h.ctl.bridge.media.contains_key(&c));
    assert!(h.ctl.bridge.calls.contains(&Call::UnloadMedia(c)));
}

#[test]
fn import_errors() {
    let (mut h, _) = harness();
    let (_, out) = import(&mut h, "notes.txt");
    assert_eq!(err(&out).code, ErrorCode::Decode);
    assert!(patches(&out).is_empty());
    let (_, out) = import(&mut h, "../etc/passwd");
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let (_, out) = import(&mut h, "missing.wav");
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    let id: MediaId = h.id();
    let out = h.send(Command::Media(MediaCommand::Import {
        id,
        source: MediaSource::Upload { upload: "u".into() },
    }));
    assert_eq!(err(&out).code, ErrorCode::Unsupported);
    let pid = h.project().id;
    assert!(h.ctl.store.file(pid, "media/notes.txt").is_none());
}

#[test]
fn browser_locations_and_listings() {
    let (mut h, _) = harness();
    let ReplyValue::Locations { locations } = h.ok(Command::Media(MediaCommand::ListLocations))
    else {
        panic!()
    };
    assert_eq!(locations.len(), 2);
    assert_eq!(locations[1].location, BrowseLocation::ProjectMedia);

    let ReplyValue::Directory { listing } = h.ok(Command::Media(MediaCommand::ListDirectory {
        location: BrowseLocation::Library { id: "lib".into() },
        path: String::new(),
    })) else {
        panic!()
    };
    assert_eq!(listing.entries[0].name, "Drums");
    assert_eq!(listing.entries[0].kind, FileKind::Directory);
    let ReplyValue::Directory { listing } = h.ok(Command::Media(MediaCommand::ListDirectory {
        location: BrowseLocation::Library { id: "lib".into() },
        path: "Drums".into(),
    })) else {
        panic!()
    };
    assert_eq!(listing.entries[0].path, "Drums/kick.wav");
    assert_eq!(listing.entries[0].kind, FileKind::Audio);

    // Empty project media folder.
    let ReplyValue::Directory { listing } = h.ok(Command::Media(MediaCommand::ListDirectory {
        location: BrowseLocation::ProjectMedia,
        path: String::new(),
    })) else {
        panic!()
    };
    assert!(listing.entries.is_empty());
    let (id, _) = import(&mut h, "Drums/kick.wav");
    let ReplyValue::Directory { listing } = h.ok(Command::Media(MediaCommand::ListDirectory {
        location: BrowseLocation::ProjectMedia,
        path: String::new(),
    })) else {
        panic!()
    };
    assert_eq!(listing.location, BrowseLocation::ProjectMedia);
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(
        listing.entries[0].path,
        format!("{id}-kick.wav"),
        "paths are relative to the location"
    );

    let out = h.send(Command::Media(MediaCommand::ListDirectory {
        location: BrowseLocation::Library { id: "lib".into() },
        path: "/abs".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}
