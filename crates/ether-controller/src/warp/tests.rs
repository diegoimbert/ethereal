use ether_core::protocol::devices::DeviceDescriptor;

use super::*;
use crate::compile::{CompileContext, builtin_descriptors, compile_graph_with};
use crate::doc::DocHost;
use crate::tx::Tx;

struct NoHost;

impl DocHost for NoHost {
    fn descriptor(&mut self, _: DeviceId, _: &DeviceKind) -> Option<DeviceDescriptor> {
        None
    }
    fn plugin_state(&mut self, _: DeviceId) -> Option<Base64Bytes> {
        None
    }
    fn instantiate_plugin(
        &mut self,
        _: DeviceId,
        _: &PluginInstance,
    ) -> CmdResult<Option<DeviceDescriptor>> {
        Ok(None)
    }
}

struct Fixture {
    ids: IdGen,
    project: Project,
    clip: ClipId,
    midi_clip: ClipId,
}

/// A project with an audio track holding one clip of `seconds` of 48 kHz media (warp as
/// given, no markers) and a MIDI clip.
fn fixture(seconds: f64, warp: WarpSettings) -> Fixture {
    let mut ids = IdGen::new(7);
    let mut project = Project::new(&mut ids, 0);
    let track = Track {
        id: ids.next(0),
        kind: TrackKind::Audio,
        name: "Audio".into(),
        color: Color(0x33_6699),
        order: OrderKey::between(None, None),
        parent: None,
        mixer: TrackMixer::default(),
        input: TrackInput::None,
        output: TrackOutput::Default,
        monitor: MonitorMode::default(),
        scale: Default::default(),
    };
    let midi_track = Track {
        id: ids.next(0),
        kind: TrackKind::Midi,
        order: OrderKey::between(Some(&track.order), None),
        ..track.clone()
    };
    let media = MediaRef {
        id: ids.next(0),
        name: "loop.wav".into(),
        file: "media/loop.wav".into(),
        sample_rate: 48_000,
        channels: 2,
        frames: (seconds * 48_000.0) as u64,
        hash: None,
    };
    let clip = Clip {
        id: ids.next(0),
        track: track.id,
        start: Beats(4.0),
        name: "loop".into(),
        color: None,
        muted: false,
        length: Beats(8.0),
        offset: Beats::ZERO,
        looping: ClipLoop {
            enabled: false,
            start: Beats::ZERO,
            end: Beats(8.0),
        },
        content: ClipContent::Audio(AudioContent {
            media: media.id,
            gain: Decibels::UNITY,
            transpose: 0.0,
            fade_in: Beats::ZERO,
            fade_out: Beats::ZERO,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp,
        }),
    };
    let midi_clip = Clip {
        id: ids.next(0),
        track: midi_track.id,
        content: ClipContent::Midi,
        ..clip.clone()
    };
    let (clip_id, midi_id) = (clip.id, midi_clip.id);
    for e in [
        Entity::Track(track),
        Entity::Track(midi_track),
        Entity::Media(media),
        Entity::Clip(clip),
        Entity::Clip(midi_clip),
    ] {
        project.apply(&Op::Insert { entity: e }).unwrap();
    }
    Fixture {
        ids,
        project,
        clip: clip_id,
        midi_clip: midi_id,
    }
}

impl Fixture {
    fn run(&mut self, c: WarpCommand) -> CmdResult<()> {
        let mut host = NoHost;
        let mut ctx = DocCtx {
            tx: Tx::new(&mut self.project),
            ids: &mut self.ids,
            now: 1,
            position: Beats::ZERO,
            host: &mut host,
            warnings: Vec::new(),
        };
        let r = command(&mut ctx, &c);
        match r {
            Ok(()) => {
                let ops = ctx.tx.finish();
                for op in &ops {
                    self.project.apply(op).unwrap();
                }
                Ok(())
            }
            Err(e) => {
                ctx.tx.rollback();
                Err(e)
            }
        }
    }

    fn markers(&self) -> Vec<(f64, f64)> {
        self.project
            .warp_markers_of(self.clip)
            .into_iter()
            .map(|m| (m.beat.0, m.source.0))
            .collect()
    }

    fn warp(&self) -> WarpSettings {
        match &self.project.clips[&self.clip].content {
            ClipContent::Audio(a) => a.warp,
            ClipContent::Midi => unreachable!(),
        }
    }

    fn desc(&self) -> Option<WarpDesc> {
        let c = &self.project.clips[&self.clip];
        let ClipContent::Audio(a) = &c.content else {
            unreachable!()
        };
        warp_desc(&self.project, c, a)
    }

    fn add(&mut self, beat: f64, source: f64) -> WarpMarkerId {
        let id = self.ids.next(2);
        self.run(WarpCommand::AddMarker {
            id,
            clip: self.clip,
            beat: Beats(beat),
            source: Seconds(source),
        })
        .unwrap();
        id
    }
}

const OFF: WarpSettings = WarpSettings {
    enabled: false,
    mode: WarpMode::Complex,
    source_bpm: None,
};

fn on(mode: WarpMode, source_bpm: Option<f64>) -> WarpSettings {
    WarpSettings {
        enabled: true,
        mode,
        source_bpm,
    }
}

#[test]
fn bpm_stub_picks_a_power_of_two_bar_count() {
    // 2 s = 1 bar at 120; 4 bars at 120 = 8 s; 7 s → 4 bars at 137.14.
    assert_eq!(detect_bpm(2.0), Some(120.0));
    assert_eq!(detect_bpm(8.0), Some(120.0));
    assert_eq!(detect_bpm(7.0), Some(137.14));
    // 1.5 s: 1 bar = 160 (excluded), 2 bars = 320 → none in range at k=0, but 3.0 s... →
    // the first power of two in range wins: 1.5 s is too short for 1 bar under 160.
    assert_eq!(detect_bpm(1.5), None);
    assert_eq!(detect_bpm(0.0), None);
    assert_eq!(detect_bpm(f64::NAN), None);
}

#[test]
fn unwarped_and_unknown_tempo_compile_to_none() {
    let f = fixture(8.0, OFF);
    assert_eq!(f.desc(), None);
    let f = fixture(8.0, on(WarpMode::Complex, None));
    assert_eq!(f.desc(), None);
}

#[test]
fn source_bpm_alone_gives_the_slope() {
    let f = fixture(8.0, on(WarpMode::Repitch, Some(100.0)));
    assert_eq!(
        f.desc(),
        Some(WarpDesc {
            mode: WarpMode::Repitch,
            markers: vec![(0.0, 0.0), (1.0, 0.6)],
        })
    );
    // One marker: the slope goes through it.
    let mut f = fixture(8.0, on(WarpMode::Complex, Some(120.0)));
    f.add(2.0, 1.5);
    assert_eq!(f.desc().unwrap().markers, vec![(2.0, 1.5), (3.0, 2.0)]);
}

#[test]
fn markers_compile_sorted_and_deduplicated() {
    let mut f = fixture(8.0, on(WarpMode::Complex, Some(120.0)));
    f.add(8.0, 3.0);
    f.add(0.0, 0.0);
    f.add(4.0, 2.5);
    f.add(4.0 + 1e-9, 9.0); // same beat: dropped
    let d = f.desc().unwrap();
    assert_eq!(d.mode, WarpMode::Complex);
    assert_eq!(d.markers.len(), 3);
    assert_eq!(d.markers[0], (0.0, 0.0));
    assert_eq!(d.markers[2], (8.0, 3.0));
    assert!(d.markers.windows(2).all(|w| w[0].0 < w[1].0));

    // The compiled graph carries it on the clip.
    let graph = compile_graph_with(
        &f.project,
        &CompileContext {
            nodes: &|_| None,
            descriptors: &builtin_descriptors,
            armed: &|_| false,
            version: 3,
        },
    );
    let clip = graph
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.id == f.clip)
        .unwrap();
    let ether_core::graph::ClipContentDesc::Audio { warp, .. } = &clip.content else {
        panic!("audio clip expected");
    };
    assert_eq!(warp.as_ref(), Some(&d));
}

#[test]
fn enabling_warp_initializes_markers_from_the_bpm_stub() {
    // 7 s of media → 4 bars at 137.14 BPM: markers at (0, 0) and (16, 7).
    let mut f = fixture(7.0, OFF);
    f.run(WarpCommand::SetWarp {
        clip: f.clip,
        warp: on(WarpMode::Complex, None),
    })
    .unwrap();
    assert_eq!(f.warp().source_bpm, Some(137.14));
    let m = f.markers();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0], (0.0, 0.0));
    assert!((m[1].0 - 7.0 * 137.14 / 60.0).abs() < 1e-9 && m[1].1 == 7.0);

    // Toggling off/on keeps the existing markers; changing the mode touches nothing else.
    f.run(WarpCommand::SetWarp {
        clip: f.clip,
        warp: OFF,
    })
    .unwrap();
    f.run(WarpCommand::SetWarp {
        clip: f.clip,
        warp: on(WarpMode::Repitch, Some(137.14)),
    })
    .unwrap();
    assert_eq!(f.markers(), m);
    assert_eq!(f.warp().mode, WarpMode::Repitch);
    assert_eq!(f.desc().unwrap().markers, m);
}

#[test]
fn enabling_warp_without_a_detectable_tempo_uses_the_given_bpm() {
    let mut f = fixture(1.0, OFF); // 1 s: no power-of-two bar count in [80, 160)
    f.run(WarpCommand::SetWarp {
        clip: f.clip,
        warp: on(WarpMode::Complex, Some(90.0)),
    })
    .unwrap();
    assert_eq!(f.markers(), vec![(0.0, 0.0), (1.5, 1.0)]);
    // ... or the project tempo at the clip start.
    let mut f = fixture(1.0, OFF);
    f.run(WarpCommand::SetWarp {
        clip: f.clip,
        warp: on(WarpMode::Complex, None),
    })
    .unwrap();
    assert_eq!(f.markers(), vec![(0.0, 0.0), (2.0, 1.0)]);
    assert_eq!(f.warp().source_bpm, Some(120.0));
}

#[test]
fn marker_commands() {
    let mut f = fixture(8.0, on(WarpMode::Complex, Some(120.0)));
    let a = f.add(0.0, 0.0);
    let b = f.add(4.0, 2.0);
    // Idempotent add.
    f.run(WarpCommand::AddMarker {
        id: b,
        clip: f.clip,
        beat: Beats(1.0),
        source: Seconds(1.0),
    })
    .unwrap();
    assert_eq!(f.markers(), vec![(0.0, 0.0), (4.0, 2.0)]);
    f.run(WarpCommand::MoveMarker {
        id: b,
        beat: Beats(3.0),
        source: Seconds(-1.0), // clamped to the media start
    })
    .unwrap();
    assert_eq!(f.markers(), vec![(0.0, 0.0), (3.0, 0.0)]);
    f.run(WarpCommand::RemoveMarker { id: a }).unwrap();
    assert_eq!(f.markers(), vec![(3.0, 0.0)]);
    assert_eq!(
        f.run(WarpCommand::RemoveMarker { id: a }).unwrap_err().code,
        ether_core::protocol::ErrorCode::NotFound
    );
    assert!(
        f.run(WarpCommand::MoveMarker {
            id: b,
            beat: Beats(f64::NAN),
            source: Seconds(0.0),
        })
        .is_err()
    );
    // MIDI clips have no warp.
    let midi = f.midi_clip;
    assert!(
        f.run(WarpCommand::SetWarp {
            clip: midi,
            warp: on(WarpMode::Complex, None),
        })
        .is_err()
    );
    let id = f.ids.next(3);
    assert!(
        f.run(WarpCommand::AddMarker {
            id,
            clip: midi,
            beat: Beats(0.0),
            source: Seconds(0.0),
        })
        .is_err()
    );
    assert_eq!(
        f.run(WarpCommand::DetectTempo { clip: f.clip })
            .unwrap_err()
            .code,
        ether_core::protocol::ErrorCode::Unsupported
    );
}

/// Shared vectors (`ether-core/src/warp/vectors.json`, also run by the engine and the UI):
/// markers + settings compile to the expected pins.
#[test]
fn shared_compile_vectors() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../../ether-core/src/warp/vectors.json")).unwrap();
    for case in doc["vectors"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let warp: WarpSettings = serde_json::from_value(case["warp"].clone()).unwrap();
        let mut f = fixture(8.0, warp);
        for m in case["markers"].as_array().unwrap() {
            f.add(m[0].as_f64().unwrap(), m[1].as_f64().unwrap());
        }
        let want: Option<Vec<(f64, f64)>> =
            serde_json::from_value(case["compiled"].clone()).unwrap();
        assert_eq!(f.desc().map(|d| d.markers), want, "{name}");
        if let Some(d) = f.desc() {
            assert_eq!(d.mode, warp.mode, "{name}");
        }
    }
}
