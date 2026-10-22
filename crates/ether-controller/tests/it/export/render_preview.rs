//! A render suspends the live engine and the transport plays the render's master so far
//! (`rendering`, CONTRACTS.md §13.19): the preview equals the exported file, playback
//! stops at the end of the rendered part, locate is clamped to it, record and play past
//! it are refused, and done / cancel / failure resume the engine.

use super::*;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::RenderingState;

fn rendering<B: EngineBridge, S: ProjectStore>(d: &Drive<B, S>) -> Option<RenderingState> {
    d.ctl.rendering.latest()
}

fn play() -> Command {
    Command::Transport(TransportCommand::Play)
}

fn locate(at: f64) -> Command {
    Command::Transport(TransportCommand::Locate {
        position: Beats(at),
    })
}

fn refused<B: EngineBridge, S: ProjectStore>(d: &mut Drive<B, S>, command: Command, key: &str) {
    let e = err(&d.send(command.clone()));
    assert_eq!(e.code, ErrorCode::InvalidState, "{command:?}");
    assert_eq!(
        e.key.as_ref().map(|k| k.id.as_str()),
        Some(key),
        "{command:?}"
    );
}

fn notices(out: &[ServerMessage]) -> Vec<String> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Notification { key: Some(k), .. } => Some(k.id),
            _ => None,
        })
        .collect()
}

/// Tick until the render's playable part reaches `beats` (the render still running).
fn render_until<B: EngineBridge, S: ProjectStore>(d: &mut Drive<B, S>, beats: f64) {
    for _ in 0..10_000 {
        if rendering(d).is_some_and(|r| r.until.0 >= beats) {
            return;
        }
        d.tick();
        assert!(
            rendering(d).is_some(),
            "the render ended before beat {beats}"
        );
    }
    panic!("the render did not reach beat {beats}");
}

/// 32 beats (16 s at 120 bpm) of the song as float WAV: long enough to stop half way.
fn long_request() -> ExportRequest {
    request(custom(0.0, 32.0), AudioContainer::Wav, BitDepth::Float32)
}

fn real() -> Drive<RealBridge, MemoryStore> {
    let mut d = Drive::new(
        RealBridge::new(OFFLINE_BLOCK),
        MemoryStore::new(),
        library(),
    );
    build_song(&mut d, false);
    d.tick(); // publish
    d
}

#[test]
fn the_transport_plays_the_render_so_far_and_the_engine_resumes_after() {
    let mut d = real();
    let job = d.render(long_request());
    assert!(
        d.ctl.bridge.handle.suspended(),
        "the live engine is suspended"
    );
    let r = rendering(&d).unwrap();
    assert!(r.preview);
    assert_eq!((r.start, r.until), (Beats(0.0), Beats(0.0)));
    assert!((r.end.0 - 32.0).abs() < 1e-6, "{r:?}");
    // Nothing is rendered yet.
    refused(&mut d, play(), "error.render.not_rendered");

    render_until(&mut d, 4.0);
    d.ok(locate(0.0));
    d.ok(play());
    // Two beats (1 s) from the live engine, which plays the preview.
    let heard = d.ctl.bridge.render(SR as usize);
    let ph = d.ctl.bridge.handle.playhead();
    assert!(ph.playing && (ph.position.0 - 2.0).abs() < 1e-3, "{ph:?}");
    // The master meter shows the preview.
    let meters = d
        .tick()
        .into_iter()
        .filter_map(|m| match m {
            ServerMessage::Meters(f) => Some(f.tracks),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    let master = d.master();
    assert!(
        meters.iter().any(|m| m.track == master && m.peak[0] > 0.05),
        "{meters:?}"
    );

    let files = match d.finish(&job).0 {
        ExportEvent::Done {
            result: ExportResult::Files { files },
            ..
        } => files,
        other => panic!("{other:?}"),
    };
    assert!(
        !d.ctl.bridge.handle.suspended(),
        "resumed when the render ends"
    );
    assert_eq!(rendering(&d), None);
    assert!(
        d.ctl
            .transport_state()
            .is_some_and(|s| s.rendering.is_none() && s.playing),
        "the playback goes on, live"
    );
    // What played is the exported file (16-bit preview).
    let file = decode(&d.read(&files[0]), "wav");
    assert!(peak(&file.channels) > 0.05);
    for (c, (heard, file)) in heard.iter().zip(&file.channels).enumerate() {
        let worst = heard
            .iter()
            .zip(file)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 2.0 / 32768.0, "channel {c}: {worst}");
    }
}

#[test]
fn playback_stops_where_the_render_is_and_locate_stays_in_the_rendered_part() {
    let mut d = real();
    let job = d.render(long_request());
    render_until(&mut d, 2.0);
    let until = rendering(&d).unwrap().until.0;
    // Past the rendered part: clamped, with a hint.
    let out = d.send(locate(20.0));
    ok(&out);
    assert_eq!(notices(&out), ["notice.render.not_rendered_yet"]);
    assert!((d.ctl.transport_state().unwrap().start_position.0 - until).abs() < 1e-9);
    // There, nothing plays yet.
    refused(&mut d, play(), "error.render.not_rendered");
    // Record waits for the render's end.
    refused(
        &mut d,
        Command::Recording(RecordingCommand::SetRecording { enabled: true }),
        "error.render.running",
    );
    // Inside it: plays up to the end of what is rendered, then stops by itself.
    d.ok(locate(until - 1.0));
    d.ok(play());
    let heard = d.ctl.bridge.render(SR as usize * 2);
    let ph = d.ctl.bridge.handle.playhead();
    assert!(!ph.playing, "stopped at the end of the rendered part");
    assert!((ph.position.0 - until).abs() < 1e-3, "{ph:?} vs {until}");
    // Half a second (one beat) played, then silence.
    let edge = SR as usize / 2 + 2;
    assert!(heard[0][edge..].iter().all(|&s| s == 0.0));
    let out = d.tick();
    assert!(notices(&out).contains(&"notice.render.preview_end".to_string()));
    assert!(!d.ctl.transport_state().unwrap().playing);
    d.ok(Command::Export(ExportCommand::Cancel { job }));
    assert!(!d.ctl.bridge.handle.suspended());
}

fn fake() -> Drive<FakeBridge, MemoryStore> {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let t = d.track(TrackKind::Midi, "Keys", None);
    d.device(t, BuiltinDevice::Synth);
    d.notes(t, 0.0, &[60]);
    d
}

/// The suspension and preview calls, in order.
fn suspension_calls(d: &Drive<FakeBridge, MemoryStore>) -> Vec<String> {
    d.ctl
        .bridge
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::Suspend(s) => Some(format!("suspend {s}")),
            Call::RenderPreview(s) => Some(format!("preview {s}")),
            _ => None,
        })
        .collect()
}

#[test]
fn cancel_resumes_and_edits_during_the_render_reach_the_document() {
    let mut d = fake();
    let t = d.track(TrackKind::Audio, "Vox", None);
    d.ok(play());
    d.ctl.bridge.calls.clear();
    let job = d.render(request(
        custom(0.0, 64.0),
        AudioContainer::Wav,
        BitDepth::Int16,
    ));
    // Live playback stops; the engine is suspended, the preview begun.
    assert!(!d.ctl.transport_state().unwrap().playing);
    assert_eq!(suspension_calls(&d), ["preview true", "suspend true"]);
    let mut seen = Vec::new();
    for _ in 0..20 {
        for e in events(&d.tick()) {
            if let Event::Transport { state } = e {
                seen.push(state.rendering.expect("rendering").until.0);
            }
        }
    }
    assert!(
        seen.len() >= 2 && seen.windows(2).all(|w| w[0] <= w[1]),
        "{seen:?}"
    );
    assert!(seen.len() <= 4, "throttled to every 100 ms: {seen:?}");
    assert!(!d.ctl.bridge.preview_audio[0].is_empty());
    // Edits apply as usual.
    d.ok(Command::Track(TrackCommand::Rename {
        id: t,
        name: "Lead".into(),
    }));
    assert_eq!(d.ctl.project().unwrap().tracks[&t].name, "Lead");
    let out = d.send(Command::Export(ExportCommand::Cancel { job }));
    assert!(
        events(&out)
            .iter()
            .any(|e| matches!(e, Event::Transport { state } if state.rendering.is_none()))
    );
    assert_eq!(
        suspension_calls(&d),
        [
            "preview true",
            "suspend true",
            "preview false",
            "suspend false"
        ]
    );
    // Back to normal: play works anywhere.
    d.ok(locate(40.0));
    d.ok(play());
}

#[test]
fn a_failed_render_resumes_the_engine() {
    let bridge = FakeBridge {
        plugins: Some(
            [(
                "fx".to_string(),
                plugin_descriptor("Fx", DeviceCategory::AudioEffect),
            )]
            .into(),
        ),
        ..Default::default()
    };
    let mut d = Drive::new(bridge, MemoryStore::new(), library());
    let t = d.track(TrackKind::Audio, "Fx", None);
    let dev: DeviceId = d.id();
    d.ok(Command::Device(DeviceCommand::Insert {
        id: dev,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "fx".into(),
            sandboxed: None,
            format: None,
            name: None,
        },
        before: None,
    }));
    d.ctl.bridge.calls.clear();
    let job = d.render(request(
        custom(0.0, 4.0),
        AudioContainer::Wav,
        BitDepth::Int16,
    ));
    assert!(matches!(d.finish(&job).0, ExportEvent::Failed { .. }));
    assert_eq!(
        suspension_calls(&d),
        [
            "preview true",
            "suspend true",
            "preview false",
            "suspend false"
        ]
    );
    assert_eq!(rendering(&d), None);
}

#[test]
fn renders_without_a_mix_to_preview_keep_the_timeline_unavailable() {
    // Stems that each need a pass of their own (a solo makes captures differ): no mix.
    let mut d = fake();
    let a = d.track(TrackKind::Midi, "A", None);
    let b = d.track(TrackKind::Midi, "B", None);
    d.ok(Command::Mixer(MixerCommand::SetSolo {
        track: a,
        solo: true,
        exclusive: false,
    }));
    let mut r = request(custom(0.0, 16.0), AudioContainer::Wav, BitDepth::Int16);
    r.mode = ExportMode::Stems { tracks: vec![a, b] };
    d.ctl.bridge.calls.clear();
    let job = d.render(r);
    assert_eq!(
        suspension_calls(&d),
        ["suspend true"],
        "suspended, no preview"
    );
    d.tick();
    let state = rendering(&d).unwrap();
    assert!(!state.preview && state.until == state.start);
    refused(&mut d, play(), "error.render.no_preview");
    d.ok(Command::Export(ExportCommand::Cancel { job }));
    assert_eq!(suspension_calls(&d), ["suspend true", "suspend false"]);

    // An engine that can't preview: the same.
    d.ctl.bridge.no_render_preview = true;
    let job = d.render(request(
        custom(0.0, 4.0),
        AudioContainer::Wav,
        BitDepth::Int16,
    ));
    assert!(!rendering(&d).unwrap().preview);
    refused(&mut d, play(), "error.render.no_preview");
    assert!(matches!(d.finish(&job).0, ExportEvent::Done { .. }));
}

#[test]
fn stems_only_renders_preview_the_mix_they_capture_from() {
    let mut d = fake();
    let a = d.track(TrackKind::Midi, "A", None);
    d.device(a, BuiltinDevice::Synth);
    d.notes(a, 0.0, &[64]);
    let keys = d
        .ctl
        .project()
        .unwrap()
        .tracks
        .values()
        .find(|t| t.name == "Keys")
        .unwrap()
        .id;
    let mut r = request(custom(0.0, 4.0), AudioContainer::Wav, BitDepth::Float32);
    r.mode = ExportMode::Stems {
        tracks: vec![a, keys],
    };
    let job = d.render(r);
    assert!(rendering(&d).unwrap().preview);
    assert!(matches!(d.finish(&job).0, ExportEvent::Done { .. }));
    let spec = d.ctl.bridge.preview.clone().unwrap();
    assert_eq!(d.ctl.bridge.preview_audio[0].len(), spec.frames);
    assert!(
        peak(&[d.ctl.bridge.preview_audio[0].clone()]) > 0.01,
        "the mix"
    );
}
