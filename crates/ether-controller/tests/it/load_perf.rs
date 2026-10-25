//! Project open time with many audio files: ticks (the controller ticks at ~60 Hz) and
//! controller time until every media is decoded and the load ended. A harness for
//! before/after numbers:
//!
//! ```text
//! cargo test --release -p ether-controller --test it load_perf:: -- --ignored --nocapture
//! ```
//! `ETHER_LOAD_FILES` (default 16) files of `ETHER_LOAD_SECONDS` (default 30) stereo 48 kHz.

use crate::common;

use std::time::{Duration, Instant};

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn open_with_many_audio_files() {
    let (files, seconds) = (env("ETHER_LOAD_FILES", 16), env("ETHER_LOAD_SECONDS", 30));
    let frames = seconds * 48_000;
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    // `ETHER_LOAD_FORMAT`: wav (default), flac or mp3.
    let format = std::env::var("ETHER_LOAD_FORMAT").unwrap_or_else(|_| "wav".into());
    for f in 0..files {
        let l = sine(48_000, 110.0 + f as f32, frames, 0.3);
        let r = sine(48_000, 220.0 + f as f32, frames, 0.3);
        lib.add_file("lib", &format!("f{f}.{format}"), encode(&format, &[l, r]));
    }
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    let project = h.create_project("Load perf");
    for f in 0..files {
        let media: MediaId = h.id();
        h.ok(Command::Media(MediaCommand::Import {
            id: media,
            source: MediaSource::Location {
                location: BrowseLocation::Library { id: "lib".into() },
                path: format!("f{f}.{format}"),
            },
        }));
        let track: TrackId = h.id();
        h.ok(Command::Track(TrackCommand::Create {
            id: track,
            kind: TrackKind::Audio,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        let id: ClipId = h.id();
        h.ok(Command::Clip(ClipCommand::CreateAudio {
            id,
            track,
            start: Beats(0.0),
            media,
        }));
    }
    h.drain_media();
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    // Reopen: everything decodes again.
    let other = h.create_project("Other");
    let _ = other;
    let t = Instant::now();
    h.ok(Command::Project(ProjectCommand::Open { id: project }));
    let mut ticks = 0;
    let mut busy = Duration::ZERO;
    while h.ctl.media_pending() && ticks < 1_000_000 {
        h.advance(16);
        let s = Instant::now();
        h.tick();
        busy += s.elapsed();
        ticks += 1;
    }
    let wall = t.elapsed();
    println!(
        "open {files} x {seconds} s stereo {format} files: {ticks} ticks until decoded (~{:.1} s at 60 Hz), controller busy {:.0} ms ({:.2} ms per tick), harness wall {:.0} ms",
        ticks as f64 / 60.0,
        busy.as_secs_f64() * 1e3,
        busy.as_secs_f64() * 1e3 / ticks.max(1) as f64,
        wall.as_secs_f64() * 1e3,
    );
}

/// `channels` at 48 kHz as a `format` file.
fn encode(format: &str, channels: &[Vec<f32>]) -> Vec<u8> {
    use ether_controller::export::encode::{FlacEncoder, Mp3Encoder, Sink};
    use ether_core::protocol::export::BitDepth;
    let finish = |sink: Sink| match sink {
        Sink::Memory(c) => c.into_inner(),
        Sink::File(_) => unreachable!(),
    };
    let block: Vec<&[f32]> = channels.iter().map(|c| c.as_slice()).collect();
    match format {
        "flac" => {
            let mut e =
                FlacEncoder::new(2, 48_000, BitDepth::Int16, Sink::memory(1 << 20)).unwrap();
            e.push(&block, 1.0).unwrap();
            finish(e.finish().unwrap())
        }
        "mp3" => {
            let mut e = Mp3Encoder::new(2, 48_000, Sink::memory(1 << 20)).unwrap();
            e.push(&block, 1.0).unwrap();
            finish(e.finish().unwrap())
        }
        _ => wav(48_000, channels),
    }
}
