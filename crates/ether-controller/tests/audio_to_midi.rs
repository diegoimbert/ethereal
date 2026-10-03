//! Audio to MIDI (`audio-to-midi`, CONTRACTS.md §13.5) through the controller: a WAV is
//! imported, placed as an audio clip and converted. The job runs from the tick in bounded
//! slices with `Progress` events, then applies one undo step (a MIDI track right below the
//! source, a clip at the source clip's position and length, notes with derived ids, an
//! optional instrument) before `Done`. Cancel, one job at a time, the clip or project going
//! away, warped / trimmed / transposed / reversed clips, and the round trip through the
//! real Poly Synth ("rendered from MIDI": precision and recall are printed).

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::audio_to_midi::{
    AudioToMidiCommand, AudioToMidiEvent, AudioToMidiMode, AudioToMidiOptions,
};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::warp::WarpCommand;
use ether_core::protocol::*;
use ether_core::{
    AudioBuffers, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_model::derive_id;

const SR: u32 = 44_100;

/// `(seconds, seconds, key)`.
type Tone = (f64, f64, u8);

fn hz(key: u8) -> f64 {
    440.0 * 2f64.powf((key as f64 - 69.0) / 12.0)
}

/// Sine notes with 3 ms ramps, mono.
fn sines(len: f64, notes: &[Tone]) -> Vec<f32> {
    let n = (len * SR as f64) as usize;
    let mut x = vec![0.0f32; n];
    for &(s, d, k) in notes {
        let (a, b) = ((s * SR as f64) as usize, (((s + d) * SR as f64) as usize).min(n));
        for (i, v) in x.iter_mut().enumerate().take(b).skip(a) {
            let t = (i - a) as f64 / SR as f64;
            let ramp = (t / 0.003).min(1.0).min((d - t).max(0.0) / 0.003);
            *v += (0.3 * ramp * (2.0 * std::f64::consts::PI * hz(k) * t).sin()) as f32;
        }
    }
    x
}

/// A short melody: C D E G (detached), then A legato into C.
fn melody() -> Vec<Tone> {
    vec![
        (0.25, 0.4, 60),
        (0.75, 0.4, 62),
        (1.25, 0.4, 64),
        (1.75, 0.4, 67),
        (2.25, 0.5, 69),
        (2.75, 0.7, 72),
    ]
}

/// A harness whose library holds `files` (name → mono samples at [`SR`]).
fn harness(files: &[(&str, Vec<f32>)]) -> Harness {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    for (name, x) in files {
        lib.add_file("lib", name, wav(SR, std::slice::from_ref(x)));
    }
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("A2M");
    h
}

/// Import `path`, put it on a new audio track at `start`. Returns (track, clip).
fn audio_clip(h: &mut Harness, path: &str, start: f64) -> (TrackId, ClipId) {
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: path.into(),
        },
    }));
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Audio,
        name: Some("Vox".into()),
        color: None,
        parent: None,
        before: None,
    }));
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: clip,
        track,
        start: Beats(start),
        media,
    }));
    h.drain_media();
    (track, clip)
}

struct Ids {
    track: TrackId,
    clip: ClipId,
    seed: NoteId,
    instrument: DeviceId,
}

fn start_cmd(h: &mut Harness, job: &str, clip: ClipId, mode: AudioToMidiMode) -> (Command, Ids) {
    let ids = Ids {
        track: h.id(),
        clip: h.id(),
        seed: h.id(),
        instrument: h.id(),
    };
    let c = Command::AudioToMidi(AudioToMidiCommand::Start {
        job: job.into(),
        clip,
        mode,
        options: AudioToMidiOptions::default(),
        track: ids.track,
        new_clip: ids.clip,
        seed_notes: ids.seed,
        instrument: Some(ids.instrument),
    });
    (c, ids)
}

fn a2m(out: &[ServerMessage]) -> Vec<AudioToMidiEvent> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::AudioToMidi { event } => Some(event),
            _ => None,
        })
        .collect()
}

/// Tick until the job ends; returns every message and the number of ticks.
fn run(h: &mut Harness) -> (Vec<ServerMessage>, usize) {
    let mut all = Vec::new();
    for ticks in 1..=2_000 {
        let out = h.tick();
        let end = a2m(&out).iter().any(|e| {
            matches!(
                e,
                AudioToMidiEvent::Done { .. }
                    | AudioToMidiEvent::Failed { .. }
                    | AudioToMidiEvent::Cancelled { .. }
            )
        });
        all.extend(out);
        if end {
            return (all, ticks);
        }
    }
    panic!("the job did not end");
}

/// Notes of `clip` in (start, pitch) order.
fn notes_of(h: &Harness, clip: ClipId) -> Vec<Note> {
    let mut v: Vec<Note> = h
        .project()
        .notes
        .values()
        .filter(|n| n.clip == clip)
        .cloned()
        .collect();
    v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.pitch.cmp(&b.pitch)));
    v
}

fn bpm(h: &Harness) -> f64 {
    h.project().tempo_map().bpm_at(Beats(0.0))
}

/// Every reference tone has a note with its key whose start is within 20 ms.
fn assert_notes(notes: &[Note], expected: &[(f64, u8)], beats_per_sec: f64) {
    assert_eq!(notes.len(), expected.len(), "{notes:#?}");
    for (n, &(beat, key)) in notes.iter().zip(expected) {
        assert_eq!(n.pitch, key, "{n:?}");
        let err_s = (n.start.0 - beat).abs() / beats_per_sec;
        assert!(err_s <= 0.020, "{n:?} vs beat {beat} ({:.1} ms)", err_s * 1000.0);
    }
}

#[test]
fn converts_a_melody_into_a_midi_track_below_in_one_undo_step() {
    let mut h = harness(&[("vox.wav", sines(3.6, &melody()))]);
    let (src, clip) = audio_clip(&mut h, "vox.wav", 2.0);
    // Another track after the source: the new one goes in between.
    let other: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: other,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let before = h.project().clone();
    let (c, ids) = start_cmd(&mut h, "job-1", clip, AudioToMidiMode::Melody);
    let out = h.send(c);
    assert_eq!(ok(&out), ReplyValue::Unit);
    assert!(patches(&out).is_empty(), "nothing changes until Done");

    let (out, _) = run(&mut h);
    let ev = a2m(&out);
    let progress: Vec<f32> = ev
        .iter()
        .filter_map(|e| match e {
            AudioToMidiEvent::Progress { progress, .. } => Some(*progress),
            _ => None,
        })
        .collect();
    assert!(progress.windows(2).all(|w| w[0] <= w[1]), "{progress:?}");
    assert_eq!(progress.last(), Some(&1.0));
    let Some(AudioToMidiEvent::Done {
        job,
        track,
        clip: new_clip,
        notes,
    }) = ev.last().cloned()
    else {
        panic!("{ev:#?}")
    };
    assert_eq!((job.as_str(), track, new_clip, notes), ("job-1", ids.track, ids.clip, 6));
    // The patch precedes Done.
    let done_at = out
        .iter()
        .position(|m| matches!(m, ServerMessage::Event(Event::AudioToMidi { event: AudioToMidiEvent::Done { .. } })))
        .unwrap();
    let patch_at = out
        .iter()
        .position(|m| matches!(m, ServerMessage::Event(Event::Patch { .. })))
        .unwrap();
    assert!(patch_at < done_at);

    let p = h.project();
    let t = &p.tracks[&ids.track];
    assert_eq!(t.kind, TrackKind::Midi);
    assert_eq!(t.name, format!("{} MIDI", p.clips[&clip].name));
    assert_eq!(t.color, p.tracks[&src].color);
    assert!(p.tracks[&src].order < t.order && t.order < p.tracks[&other].order);
    let c = &p.clips[&ids.clip];
    assert_eq!((c.track, c.start, c.length), (ids.track, Beats(2.0), p.clips[&clip].length));
    let dev = &p.devices[&ids.instrument];
    assert_eq!(dev.track, ids.track);
    assert!(matches!(
        &dev.kind,
        DeviceKind::Builtin { device: BuiltinDevice::PolySynth }
    ));
    // Pitches exact, onsets within 20 ms, ids derived in (start, pitch) order.
    let bps = bpm(&h) / 60.0;
    let notes = notes_of(&h, ids.clip);
    let expected: Vec<(f64, u8)> = melody().iter().map(|&(s, _, k)| (s * bps, k)).collect();
    assert_notes(&notes, &expected, bps);
    for (i, n) in notes.iter().enumerate() {
        assert_eq!(n.id, derive_id(ids.seed, i as u32));
    }

    // One undo step.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(notes_of(&h, ids.clip).len(), 6);
}

#[test]
fn work_is_bounded_per_tick_with_progress() {
    // 40 s of audio: many ticks, each one reporting progress.
    let mut tones = Vec::new();
    for i in 0..40 {
        tones.push((i as f64 + 0.1, 0.5, 60 + (i % 12) as u8));
    }
    let mut h = harness(&[("long.wav", sines(40.5, &tones))]);
    let (_, clip) = audio_clip(&mut h, "long.wav", 0.0);
    let (c, ids) = start_cmd(&mut h, "j", clip, AudioToMidiMode::Melody);
    h.ok(c);
    let (out, ticks) = run(&mut h);
    let progress = a2m(&out)
        .iter()
        .filter(|e| matches!(e, AudioToMidiEvent::Progress { .. }))
        .count();
    assert!(ticks >= 8, "{ticks} ticks");
    assert!(progress >= ticks - 1, "{progress} progress events in {ticks} ticks");
    assert_eq!(notes_of(&h, ids.clip).len(), 40);
}

#[test]
fn cancel_one_job_at_a_time_and_clip_or_project_going_away() {
    let x = sines(20.0, &[(0.5, 10.0, 60)]);
    let mut h = harness(&[("a.wav", x)]);
    let (_, clip) = audio_clip(&mut h, "a.wav", 0.0);
    let before = h.project().clone();

    // One at a time.
    let (c, _) = start_cmd(&mut h, "j1", clip, AudioToMidiMode::Melody);
    h.ok(c);
    let (c2, _) = start_cmd(&mut h, "j2", clip, AudioToMidiMode::Drums);
    assert_eq!(err(&h.send(c2)).code, ErrorCode::InvalidState);
    h.tick();
    // Cancel: `Cancelled`, the document is untouched, nothing more happens.
    let out = h.send(Command::AudioToMidi(AudioToMidiCommand::Cancel { job: "j1".into() }));
    assert_eq!(
        a2m(&out),
        vec![AudioToMidiEvent::Cancelled { job: "j1".into() }]
    );
    for _ in 0..5 {
        assert!(a2m(&h.tick()).is_empty());
    }
    assert_eq!(h.project(), &before);
    // Cancelling an unknown (or finished) job is a no-op.
    let out = h.send(Command::AudioToMidi(AudioToMidiCommand::Cancel { job: "nope".into() }));
    assert!(a2m(&out).is_empty());

    // The clip is deleted mid-job.
    let (c, _) = start_cmd(&mut h, "j3", clip, AudioToMidiMode::Melody);
    h.ok(c);
    h.tick();
    h.ok(Command::Clip(ClipCommand::Delete { ids: vec![clip] }));
    let (out, _) = run(&mut h);
    assert_eq!(a2m(&out), vec![AudioToMidiEvent::Cancelled { job: "j3".into() }]);

    // The project is closed (another one opened) mid-job.
    h.ok(Command::Edit(EditCommand::Undo));
    let (c, _) = start_cmd(&mut h, "j4", clip, AudioToMidiMode::Melody);
    h.ok(c);
    h.tick();
    let other = h.project_id();
    h.ok(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    }));
    let (out, _) = run(&mut h);
    assert_eq!(a2m(&out), vec![AudioToMidiEvent::Cancelled { job: "j4".into() }]);
}

#[test]
fn start_is_validated() {
    let mut h = harness(&[("a.wav", sines(1.0, &[(0.1, 0.5, 60)]))]);
    let (src, clip) = audio_clip(&mut h, "a.wav", 0.0);
    let start = |job: &str, clip, track, options| {
        Command::AudioToMidi(AudioToMidiCommand::Start {
            job: job.into(),
            clip,
            mode: AudioToMidiMode::Melody,
            options,
            track,
            new_clip: ClipId::NIL,
            seed_notes: NoteId::NIL,
            instrument: None,
        })
    };
    let fresh: TrackId = h.id();
    let d = AudioToMidiOptions::default();
    let code = |h: &mut Harness, c| err(&h.send(c)).code;
    assert_eq!(code(&mut h, start("", clip, fresh, d.clone())), ErrorCode::InvalidArgument);
    let missing: ClipId = h.id();
    assert_eq!(code(&mut h, start("j", missing, fresh, d.clone())), ErrorCode::NotFound);
    assert_eq!(code(&mut h, start("j", clip, src, d.clone())), ErrorCode::InvalidArgument);
    for bad in [
        AudioToMidiOptions {
            sensitivity: 1.5,
            ..d.clone()
        },
        AudioToMidiOptions {
            min_pitch: 80,
            max_pitch: 70,
            ..d.clone()
        },
        AudioToMidiOptions {
            min_duration: Seconds(f64::NAN),
            ..d.clone()
        },
        AudioToMidiOptions {
            kick_key: 200,
            ..d.clone()
        },
    ] {
        assert_eq!(code(&mut h, start("j", clip, fresh, bad)), ErrorCode::InvalidArgument);
    }
    // No project open.
    let mut empty = Harness::new();
    assert_eq!(code(&mut empty, start("j", clip, fresh, d)), ErrorCode::InvalidState);
}

#[test]
fn notes_map_through_warp_window_transpose_and_reverse() {
    let mut h = harness(&[("vox.wav", sines(3.6, &melody()))]);
    let (_, clip) = audio_clip(&mut h, "vox.wav", 0.0);
    let bps = bpm(&h) / 60.0;

    // Warped: 1 source second per beat (markers 0 → 0 s and 4 → 4 s).
    for (beat, source) in [(0.0, 0.0), (4.0, 4.0)] {
        let id: WarpMarkerId = h.id();
        h.ok(Command::Warp(WarpCommand::AddMarker {
            id,
            clip,
            beat: Beats(beat),
            source: Seconds(source),
        }));
    }
    h.ok(Command::Warp(WarpCommand::SetWarp {
        clip,
        warp: WarpSettings {
            enabled: true,
            mode: WarpMode::Complex,
            source_bpm: None,
        },
    }));
    // Window: skip the first beat (= first note), 2 beats long → notes 2..=4 of 6.
    h.ok(Command::Clip(ClipCommand::SetBounds {
        id: clip,
        start: Beats(8.0),
        length: Beats(2.0),
        offset: Beats(0.5),
    }));
    let (c, ids) = start_cmd(&mut h, "w", clip, AudioToMidiMode::Melody);
    h.ok(c);
    run(&mut h);
    let notes = notes_of(&h, ids.clip);
    // 0.75 s → beat 0.75 - 0.5, 1.25 s → 0.75, 1.75 s → 1.25, 2.25 s → 1.75.
    let expected = [(0.25, 62), (0.75, 64), (1.25, 67), (1.75, 69)];
    assert_notes(&notes, &expected, 1.0);
    // Cut at the clip end.
    assert!(notes.iter().all(|n| n.start.0 + n.duration.0 <= 2.0 + 1e-9));
    let c = &h.project().clips[&ids.clip];
    assert_eq!((c.start, c.length), (Beats(8.0), Beats(2.0)));

    // Unwarped, transposed +12 (repitch: an octave up, twice as fast).
    let (_, clip) = audio_clip(&mut h, "vox.wav", 0.0);
    h.ok(Command::Clip(ClipCommand::SetTranspose {
        id: clip,
        semitones: 12.0,
    }));
    let (c, ids) = start_cmd(&mut h, "t", clip, AudioToMidiMode::Melody);
    h.ok(c);
    run(&mut h);
    let expected: Vec<(f64, u8)> = melody()
        .iter()
        .map(|&(s, _, k)| (s / 2.0 * bps, k + 12))
        .collect();
    assert_notes(&notes_of(&h, ids.clip), &expected, bps);

    // Reversed: the last note comes first; a note's start is the end of the tone.
    let (_, clip) = audio_clip(&mut h, "vox.wav", 0.0);
    h.ok(Command::Clip(ClipCommand::SetReversed {
        id: clip,
        reversed: true,
    }));
    let (c, ids) = start_cmd(&mut h, "r", clip, AudioToMidiMode::Melody);
    h.ok(c);
    run(&mut h);
    let notes = notes_of(&h, ids.clip);
    let keys: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
    assert_eq!(keys, [72, 69, 67, 64, 62, 60]);
    let (s, d, _) = melody()[5];
    let want = (3.6 - (s + d)) * bps;
    assert!((notes[0].start.0 - want).abs() / bps < 0.03, "{:?} vs {want}", notes[0]);
}

#[test]
fn drums_and_harmony_modes() {
    // Kick (decaying 60 Hz sine burst) and hats (noise bursts) alternating, then chords.
    let mut x = vec![0.0f32; (2.2 * SR as f64) as usize];
    let mut seed = 1u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut hits = Vec::new();
    for i in 0..8 {
        let t = 0.1 + i as f64 * 0.25;
        let a = (t * SR as f64) as usize;
        if i % 2 == 0 {
            hits.push((t, 36));
            for j in 0..(0.2 * SR as f64) as usize {
                let tt = j as f64 / SR as f64;
                x[a + j] += (0.6 * (2.0 * std::f64::consts::PI * 60.0 * tt).sin() * (-tt / 0.08).exp()) as f32;
            }
        } else {
            hits.push((t, 42));
            let mut prev = 0.0f32;
            for j in 0..(0.05 * SR as f64) as usize {
                let tt = j as f64 / SR as f64;
                let n = noise();
                let hp = n - prev; // crude high-pass
                prev = n;
                x[a + j] += (0.3 * hp as f64 * (-tt / 0.015).exp()) as f32;
            }
        }
    }
    let chords = sines(
        3.0,
        &[
            (0.2, 1.2, 60),
            (0.2, 1.2, 64),
            (0.2, 1.2, 67),
            (1.6, 1.2, 57),
            (1.6, 1.2, 60),
            (1.6, 1.2, 65),
        ],
    );
    let mut h = harness(&[("drums.wav", x), ("keys.wav", chords)]);
    let bps = bpm(&h) / 60.0;

    let (_, clip) = audio_clip(&mut h, "drums.wav", 0.0);
    let (c, ids) = start_cmd(&mut h, "d", clip, AudioToMidiMode::Drums);
    h.ok(c);
    run(&mut h);
    let expected: Vec<(f64, u8)> = hits.iter().map(|&(t, k)| (t * bps, k)).collect();
    assert_notes(&notes_of(&h, ids.clip), &expected, bps);
    assert!(matches!(
        &h.project().devices[&ids.instrument].kind,
        DeviceKind::Builtin { device: BuiltinDevice::DrumRack }
    ));

    let (_, clip) = audio_clip(&mut h, "keys.wav", 0.0);
    let (c, ids) = start_cmd(&mut h, "k", clip, AudioToMidiMode::Harmony);
    h.ok(c);
    run(&mut h);
    let expected: Vec<(f64, u8)> = [(0.2, 60), (0.2, 64), (0.2, 67), (1.6, 57), (1.6, 60), (1.6, 65)]
        .iter()
        .map(|&(t, k)| (t * bps, k))
        .collect();
    assert_notes(&notes_of(&h, ids.clip), &expected, bps);
}

// ─── round trip through the Poly Synth ─────────────────────────────────────────────────

/// Render `(seconds, seconds, key)` notes with the factory-default Poly Synth (mono mix).
fn poly_synth(len: f64, notes: &[Tone]) -> Vec<f32> {
    let sr = SR as f32;
    let mut synth = ether_devices::poly_synth::PolySynth::new();
    synth.prepare(&PrepareConfig {
        sample_rate: sr,
        max_block_size: 256,
        max_events_per_block: 256,
    });
    let frames = (len * SR as f64) as usize;
    let mut events: Vec<(u64, EventKind)> = Vec::new();
    for (i, &(s, d, k)) in notes.iter().enumerate() {
        let id = i as u32;
        events.push((
            (s * SR as f64) as u64,
            EventKind::NoteOn {
                note_id: id,
                channel: 0,
                key: k,
                velocity: 0.8,
            },
        ));
        events.push((
            ((s + d) * SR as f64) as u64,
            EventKind::NoteOff {
                note_id: id,
                channel: 0,
                key: k,
                velocity: 0.0,
            },
        ));
    }
    events.sort_by_key(|e| e.0);
    let mut out = vec![0.0f32; frames];
    let mut out_events = EventBuffer::with_capacity(64);
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    let mut done = 0;
    while done < frames {
        let n = 256.min(frames - done);
        let t0 = done as u64;
        let evs: Vec<ProcessEvent> = events
            .iter()
            .filter(|e| e.0 >= t0 && e.0 < t0 + n as u64)
            .map(|e| ProcessEvent {
                offset: (e.0 - t0) as u32,
                kind: e.1,
            })
            .collect();
        let transport = TransportInfo {
            playing: true,
            sample_time: t0,
            bpm: 120.0,
            beats_per_sample: 2.0 / f64::from(sr),
            position: t0 as f64 * 2.0 / f64::from(sr),
            ..TransportInfo::STOPPED
        };
        out_events.clear();
        let mut outs: [&mut [f32]; 2] = [&mut l[..n], &mut r[..n]];
        let mut ctx = ProcessContext {
            sample_rate: sr,
            frames: n,
            transport: &transport,
            events: &evs,
            out_events: &mut out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &[],
            outputs: &mut outs,
        };
        synth.process(&mut ctx, &mut buffers);
        for j in 0..n {
            out[done + j] = 0.5 * (l[j] + r[j]);
        }
        done += n;
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    out.iter().map(|v| v * 0.7 / peak).collect()
}

/// Onset-only note precision/recall (same key, onset within `tol` seconds, one-to-one).
fn precision_recall(notes: &[Note], reference: &[(f64, u8)], bps: f64, tol: f64) -> (f64, f64) {
    let mut used = vec![false; notes.len()];
    let mut matched = 0;
    for &(beat, key) in reference {
        if let Some(i) = (0..notes.len()).find(|&i| {
            !used[i] && notes[i].pitch == key && (notes[i].start.0 - beat).abs() / bps <= tol
        }) {
            used[i] = true;
            matched += 1;
        }
    }
    (
        matched as f64 / notes.len().max(1) as f64,
        matched as f64 / reference.len().max(1) as f64,
    )
}

#[test]
fn poly_synth_round_trip() {
    let lead: Vec<Tone> = [
        (0.2, 60),
        (0.6, 63),
        (1.0, 67),
        (1.4, 70),
        (1.8, 72),
        (2.2, 70),
        (2.6, 67),
        (3.0, 65),
        (3.4, 55),
        (3.8, 48),
    ]
    .iter()
    .map(|&(s, k)| (s, 0.3, k))
    .collect();
    let mut pads = Vec::new();
    for (i, chord) in [[48u8, 55, 64], [45, 57, 60], [41, 57, 65], [43, 59, 62]]
        .iter()
        .enumerate()
    {
        for &k in chord {
            pads.push((0.2 + i as f64 * 0.9, 0.7, k));
        }
    }
    let mut h = harness(&[
        ("lead.wav", poly_synth(4.5, &lead)),
        ("pads.wav", poly_synth(4.0, &pads)),
    ]);
    let bps = bpm(&h) / 60.0;
    eprintln!("| Poly Synth fixture | ref | det | P@20 | R@20 | P@50 | R@50 |");
    let mut scores = Vec::new();
    for (path, mode, tones) in [
        ("lead.wav", AudioToMidiMode::Melody, &lead),
        ("pads.wav", AudioToMidiMode::Harmony, &pads),
    ] {
        let (_, clip) = audio_clip(&mut h, path, 0.0);
        let (c, ids) = start_cmd(&mut h, path, clip, mode);
        h.ok(c);
        run(&mut h);
        let notes = notes_of(&h, ids.clip);
        let reference: Vec<(f64, u8)> = tones.iter().map(|&(s, _, k)| (s * bps, k)).collect();
        let (p20, r20) = precision_recall(&notes, &reference, bps, 0.020);
        let (p50, r50) = precision_recall(&notes, &reference, bps, 0.050);
        eprintln!(
            "| {path:<18} | {:>3} | {:>3} | {p20:.3} | {r20:.3} | {p50:.3} | {r50:.3} |",
            reference.len(),
            notes.len()
        );
        scores.push((path, p50, r50));
    }
    for (path, p, r) in scores {
        assert!(p >= 0.9 && r >= 0.9, "{path}: P/R@50 {p:.3}/{r:.3}");
    }
}
