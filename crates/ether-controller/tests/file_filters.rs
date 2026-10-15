//! Library file filters (`base-145`; CONTRACTS.md §12.8) through the controller with an
//! in-memory library (the store the web host mirrors): `Media::ListDirectory` and the index
//! show only supported files by default, `SetFileFilter` / `ResetFileFilter` re-filter both
//! at once (with `FileFilterChanged`), `CheckFileFilter` reports bad rules, per-path verdicts
//! and the library count, and the filter persists in the user library.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::Library;
use ether_core::protocol::browser::*;
use ether_core::protocol::media::{BrowseLocation, MediaCommand};
use ether_core::protocol::{Command, ErrorCode, Event, ReplyValue, ServerMessage};

const LIB: &str = "lib";
const USER: &str = "user";
const FILE: &str = "Settings/library-filters.json";

fn library() -> MemoryLibrary {
    let mut lib = MemoryLibrary::new().with_user_root(USER);
    lib.add_root(LIB, "Samples");
    let wav = wav(44_100, &[sine(44_100, 220.0, 4_410, 0.5)]);
    lib.add_file(LIB, "Kick.wav", wav.clone());
    lib.add_file(LIB, "Kick.wav.asd", b"asd".to_vec());
    lib.add_file(LIB, "Song.als", b"als".to_vec());
    lib.add_file(LIB, "Song.flp", b"flp".to_vec());
    lib.add_file(LIB, "Thumbs.db", b"db".to_vec());
    lib.add_file(LIB, "notes.txt", b"x".to_vec());
    lib.add_file(LIB, "Groove.MID", b"MThd".to_vec());
    lib.add_file(LIB, ".DS_Store", b"x".to_vec());
    lib.add_file(LIB, "Loops/Break.wav", wav.clone());
    lib.add_file(LIB, "Backup/Old.wav", wav);
    lib
}

fn harness() -> Harness {
    Harness::with(FakeBridge::default(), library(), Default::default())
}

fn browser(h: &mut Harness, c: BrowserCommand) -> Vec<ServerMessage> {
    h.send(Command::Browser(c))
}

fn names(h: &mut Harness, path: &str) -> Vec<String> {
    match h.ok(Command::Media(MediaCommand::ListDirectory {
        location: BrowseLocation::Library { id: LIB.into() },
        path: path.into(),
    })) {
        ReplyValue::Directory { listing } => listing.entries.into_iter().map(|e| e.name).collect(),
        other => panic!("{other:?}"),
    }
}

fn get(h: &mut Harness) -> (FileFilter, FileFilter) {
    match ok(&browser(h, BrowserCommand::GetFileFilter)) {
        ReplyValue::BrowserFileFilter { filter, defaults } => (filter, defaults),
        other => panic!("{other:?}"),
    }
}

fn check(h: &mut Harness, filter: Option<FileFilter>, paths: &[&str]) -> FileFilterCheck {
    match ok(&browser(
        h,
        BrowserCommand::CheckFileFilter {
            filter,
            paths: paths.iter().map(|p| p.to_string()).collect(),
        },
    )) {
        ReplyValue::BrowserFileFilterCheck { check } => check,
        other => panic!("{other:?}"),
    }
}

/// Tick until the index is idle.
fn index(h: &mut Harness) {
    let mut quiet = 0;
    for _ in 0..10_000 {
        h.advance(20);
        let busy = events(&h.tick())
            .iter()
            .any(|e| matches!(e, Event::Browser { .. }));
        quiet = if busy { 0 } else { quiet + 1 };
        if quiet > 150 {
            return;
        }
    }
    panic!("indexing never settled");
}

fn indexed(h: &mut Harness) -> Vec<String> {
    let query = BrowserQuery {
        text: String::new(),
        kinds: vec![],
        tags: vec![],
        favourites_only: false,
        roots: vec![LIB.into()],
        folder: None,
        device: None,
        sort: BrowserSort::Name,
        offset: 0,
        limit: 200,
    };
    match ok(&browser(h, BrowserCommand::Query { query })) {
        ReplyValue::BrowserPage { page } => {
            let mut paths: Vec<String> = page.items.into_iter().map(|i| i.path).collect();
            paths.sort();
            paths
        }
        other => panic!("{other:?}"),
    }
}

fn changed(out: &[ServerMessage]) -> Vec<FileFilter> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Browser {
                event: BrowserEvent::FileFilterChanged { filter },
            } => Some(filter),
            _ => None,
        })
        .collect()
}

fn with_rule(mut f: FileFilter, white: &[&str], black: &[&str]) -> FileFilter {
    f.whitelist.extend(white.iter().map(|s| s.to_string()));
    f.blacklist.extend(black.iter().map(|s| s.to_string()));
    f
}

#[test]
fn listings_show_supported_files_by_default() {
    let mut h = harness();
    // Without the browser ever opened: the folder view is filtered already.
    assert_eq!(
        names(&mut h, ""),
        ["Backup", "Loops", "Groove.MID", "Kick.wav"]
    );
    let (filter, defaults) = get(&mut h);
    assert_eq!(filter, defaults);
    assert!(
        h.ctl.library.read(USER, FILE).is_err(),
        "defaults are not written"
    );
}

#[test]
fn set_filters_listings_and_index_at_once_and_reset_restores() {
    let mut h = harness();
    browser(&mut h, BrowserCommand::ListRoots);
    index(&mut h);
    assert_eq!(
        indexed(&mut h),
        [
            "Backup/Old.wav",
            "Groove.MID",
            "Kick.wav",
            "Loops/Break.wav"
        ]
    );

    // Show Ableton projects; hide the Backup folder and anything with "kick".
    let (_, defaults) = get(&mut h);
    let filter = with_rule(defaults.clone(), &[r"\.als$"], &["^backup$", "kick"]);
    let out = browser(
        &mut h,
        BrowserCommand::SetFileFilter {
            filter: filter.clone(),
        },
    );
    assert_eq!(
        ok(&out),
        ReplyValue::BrowserFileFilter {
            filter: filter.clone(),
            defaults: defaults.clone(),
        }
    );
    assert_eq!(changed(&out), std::slice::from_ref(&filter));
    assert!(
        events(&out).contains(&Event::Browser {
            event: BrowserEvent::IndexChanged
        }),
        "the browser re-queries"
    );
    assert_eq!(names(&mut h, ""), ["Loops", "Groove.MID", "Song.als"]);
    // Hidden items leave the index right away.
    assert_eq!(indexed(&mut h), ["Groove.MID", "Loops/Break.wav"]);
    // Stored in the user library.
    let stored: FileFilter =
        serde_json::from_slice(&h.ctl.library.read(USER, FILE).unwrap()).unwrap();
    assert_eq!(stored, filter);

    // Reset: defaults back, the stored file gone, the index rescans the files it shows again.
    let out = browser(&mut h, BrowserCommand::ResetFileFilter);
    assert_eq!(changed(&out), std::slice::from_ref(&defaults));
    assert!(h.ctl.library.read(USER, FILE).is_err());
    assert_eq!(
        names(&mut h, ""),
        ["Backup", "Loops", "Groove.MID", "Kick.wav"]
    );
    index(&mut h);
    assert_eq!(
        indexed(&mut h),
        [
            "Backup/Old.wav",
            "Groove.MID",
            "Kick.wav",
            "Loops/Break.wav"
        ]
    );
}

#[test]
fn empty_whitelist_shows_everything_but_the_blacklist() {
    let mut h = harness();
    let filter = FileFilter {
        whitelist: vec![],
        blacklist: vec![r"\.asd$".into()],
    };
    ok(&browser(&mut h, BrowserCommand::SetFileFilter { filter }));
    assert_eq!(
        names(&mut h, ""),
        [
            "Backup",
            "Loops",
            "Groove.MID",
            "Kick.wav",
            "Song.als",
            "Song.flp",
            "Thumbs.db",
            "notes.txt",
        ],
        "hidden files never show"
    );
}

#[test]
fn invalid_rules_are_rejected_and_reported() {
    let mut h = harness();
    let (_, defaults) = get(&mut h);
    let bad = with_rule(defaults.clone(), &["(unclosed"], &[]);
    let e = err(&browser(
        &mut h,
        BrowserCommand::SetFileFilter {
            filter: bad.clone(),
        },
    ));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert!(e.message.starts_with("whitelist rule 4:"), "{}", e.message);
    assert_eq!(get(&mut h).0, defaults, "nothing stored");

    let c = check(&mut h, Some(bad), &["Kick.wav", "Song.als"]);
    assert_eq!(c.errors.len(), 1);
    assert_eq!(
        (c.errors[0].list, c.errors[0].index),
        (FileFilterList::Whitelist, 3)
    );
    // The good rules still answer.
    assert_eq!(c.visible, [true, false]);
}

#[test]
fn check_answers_paths_and_counts_the_library() {
    let mut h = harness();
    // Before any scan: no library count.
    let c = check(
        &mut h,
        None,
        &["Drums/Kick.wav", "Song.als", ".git/x.wav", "a/Thumbs.db"],
    );
    assert!(c.errors.is_empty());
    assert_eq!(c.visible, [true, false, false, false]);
    assert_eq!(c.library, None);
    index(&mut h);
    // Every file the scan saw (dotfiles excepted by the scan itself), user library included.
    let c = check(&mut h, None, &[]);
    let lib = c.library.expect("scanned");
    assert_eq!(lib.shown, 4);
    assert!(lib.total >= 9, "{lib:?}");
    let all = FileFilter {
        whitelist: vec![],
        blacklist: vec![],
    };
    let c = check(&mut h, Some(all), &[]);
    assert_eq!(c.library.unwrap().shown, lib.total);
}

#[test]
fn stored_filter_survives_a_restart_and_a_bad_file_is_ignored() {
    let mut h = harness();
    let (_, defaults) = get(&mut h);
    let filter = with_rule(defaults.clone(), &[r"\.txt$"], &[]);
    ok(&browser(
        &mut h,
        BrowserCommand::SetFileFilter {
            filter: filter.clone(),
        },
    ));
    let lib = std::mem::take(&mut h.ctl.library);
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    assert_eq!(get(&mut h).0, filter);
    assert!(names(&mut h, "").contains(&"notes.txt".to_string()));

    h.ctl
        .library
        .write_file(USER, FILE, br#"{"whitelist":["("],"blacklist":[]}"#)
        .unwrap();
    let lib = std::mem::take(&mut h.ctl.library);
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    assert_eq!(get(&mut h).0, defaults);
}

#[test]
fn without_a_user_library_the_filter_lasts_the_session() {
    let mut lib = MemoryLibrary::new();
    lib.add_root(LIB, "Samples");
    lib.add_file(LIB, "Song.als", b"als".to_vec());
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    assert!(names(&mut h, "").is_empty());
    let filter = FileFilter {
        whitelist: vec![r"\.als$".into()],
        blacklist: vec![],
    };
    ok(&browser(
        &mut h,
        BrowserCommand::SetFileFilter {
            filter: filter.clone(),
        },
    ));
    assert_eq!(names(&mut h, ""), ["Song.als"]);
    assert_eq!(get(&mut h).0, filter);
}
