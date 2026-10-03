//! Browser v2 query latency over a 50k-item index (`browser-v2` acceptance: queries under
//! 50 ms for 50k items). Indexes a generated in-memory library through the controller
//! (bounded work per tick), then times representative queries end to end (`handle`).
//!
//! The 50 ms bound is asserted in optimized builds (`cargo test --release -p
//! ether-controller --test browser_bench -- --nocapture` prints the timings); debug builds
//! only check a loose bound, since they are ~20x slower.

mod common;

use std::time::{Duration, Instant};

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::browser::*;
use ether_core::protocol::{Command, ReplyValue};

const ITEMS: usize = 50_000;
const ROOTS: usize = 10;
const DIRS: usize = 50;

const WORDS: [&str; 16] = [
    "Kick", "Snare", "Hat", "Clap", "Bass", "Pad", "Lead", "Chord", "Vox", "Perc", "Tom", "Ride",
    "Crash", "Fx", "Riser", "Pluck",
];
const KEYS: [&str; 6] = ["Am", "C#m", "Fmaj", "Ebm", "G", "Dmin"];

/// (root, path) of generated item `i`.
fn item(i: usize) -> (String, String) {
    let dir = (i / ROOTS) % DIRS;
    let a = WORDS[i % WORDS.len()];
    let b = WORDS[(i / 7) % WORDS.len()];
    let loops = if dir % 3 == 0 { "Loops/" } else { "" };
    let tempo = 80 + (i % 90);
    let key = KEYS[i % KEYS.len()];
    (
        format!("root{}", i % ROOTS),
        format!("Pack {dir}/{loops}{a} {b} {key} {tempo} {i}.wav"),
    )
}

fn library() -> MemoryLibrary {
    let mut lib = MemoryLibrary::new().with_user_root("user");
    for i in 0..ITEMS {
        let (root, path) = item(i);
        lib.add_file(&root, &path, vec![0; 4]);
    }
    lib
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
        limit: 100,
    }
}

#[test]
fn queries_over_50k_items_are_fast() {
    let mut h = Harness::with(FakeBridge::default(), library(), Default::default());
    h.send(Command::Browser(BrowserCommand::ListRoots));
    let start = Instant::now();
    let mut slowest_tick = Duration::ZERO;
    let mut ticks = 0;
    loop {
        h.advance(20);
        let t = Instant::now();
        h.tick();
        slowest_tick = slowest_tick.max(t.elapsed());
        ticks += 1;
        let total = match h.ok(Command::Browser(BrowserCommand::Query {
            query: BrowserQuery { limit: 1, ..q("") },
        })) {
            ReplyValue::BrowserPage { page } => page.total,
            other => panic!("{other:?}"),
        };
        // + factory presets.
        if total as usize >= ITEMS {
            break;
        }
        assert!(ticks < 100_000, "indexing never finished");
    }
    println!(
        "indexed {ITEMS} items in {ticks} ticks, {:?} (slowest tick {slowest_tick:?})",
        start.elapsed()
    );
    // Let the probe pass finish too (headers of fake files fail fast).
    let mut slowest_after = Duration::ZERO;
    for _ in 0..2000 {
        h.advance(20);
        let t = Instant::now();
        h.tick();
        slowest_after = slowest_after.max(t.elapsed());
    }
    println!("probe pass + persistence: slowest tick {slowest_after:?}");
    let (root, path) = item(3);
    h.ok(Command::Browser(BrowserCommand::SetFavourite {
        item: format!("{root}/{path}"),
        favourite: true,
    }));

    let queries = [
        ("everything by name", q("")),
        ("one word", q("kick")),
        ("two words", q("kick loops")),
        (
            "common letter, relevance",
            BrowserQuery {
                sort: BrowserSort::Relevance,
                ..q("a")
            },
        ),
        (
            "deep page",
            BrowserQuery {
                offset: 40_000,
                ..q("")
            },
        ),
        (
            "bpm sort",
            BrowserQuery {
                sort: BrowserSort::Bpm,
                kinds: vec![LibraryItemKind::Audio],
                ..q("")
            },
        ),
        (
            "recent sort",
            BrowserQuery {
                sort: BrowserSort::Recent,
                ..q("")
            },
        ),
        (
            "duration sort",
            BrowserQuery {
                sort: BrowserSort::Duration,
                ..q("pad")
            },
        ),
        (
            "root + folder",
            BrowserQuery {
                roots: vec!["root4".into()],
                folder: Some("Pack 7".into()),
                ..q("")
            },
        ),
        (
            "favourites",
            BrowserQuery {
                favourites_only: true,
                ..q("")
            },
        ),
        ("no match", q("zzzz")),
    ];
    let bound = Duration::from_millis(50);
    for (what, query) in queries {
        // Best of 3 (the machine is shared).
        let mut best = Duration::MAX;
        let mut total = 0;
        for _ in 0..3 {
            let t = Instant::now();
            let reply = h.ok(Command::Browser(BrowserCommand::Query {
                query: query.clone(),
            }));
            best = best.min(t.elapsed());
            if let ReplyValue::BrowserPage { page } = reply {
                total = page.total;
            }
        }
        println!("{what:>28}: {best:>10.2?} ({total} matches)");
        assert!(best < bound, "{what}: {best:?} >= {bound:?}");
    }
}
