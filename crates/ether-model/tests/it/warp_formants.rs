//! `WarpMode::ComplexPlus` and Complex+'s formant settings in documents: defaults are not
//! written (documents without them keep re-saving byte for byte), other values round-trip.

use ether_model::*;

#[test]
fn default_formants_are_not_written() {
    let w = WarpSettings::default();
    let json = serde_json::to_value(w).unwrap();
    assert!(json.get("formants").is_none(), "{json}");
    // A document without the field reads the defaults.
    let back: WarpSettings =
        serde_json::from_str(r#"{"enabled":true,"mode":"Complex","source_bpm":120.0}"#).unwrap();
    assert_eq!(back.formants, WarpFormants::DEFAULT);
    assert_eq!(
        WarpFormants::DEFAULT,
        WarpFormants {
            amount: 1.0,
            shift: 0.0,
            envelope: 128.0
        }
    );
}

#[test]
fn complex_plus_and_formants_round_trip() {
    let w = WarpSettings {
        enabled: true,
        mode: WarpMode::ComplexPlus,
        source_bpm: Some(98.0),
        formants: WarpFormants {
            amount: 0.6,
            shift: -2.5,
            envelope: 64.0,
        },
    };
    let json = serde_json::to_string(&w).unwrap();
    assert!(json.contains(r#""mode":"ComplexPlus""#), "{json}");
    assert_eq!(serde_json::from_str::<WarpSettings>(&json).unwrap(), w);
    // A partial object fills the rest with the defaults.
    let partial: WarpFormants = serde_json::from_str(r#"{"amount":0.25}"#).unwrap();
    assert_eq!(partial.envelope, 128.0);
    assert!(WarpMode::ComplexPlus.stretches() && WarpMode::Complex.stretches());
    assert!(!WarpMode::Repitch.stretches());
}

#[test]
fn formants_clamp_to_their_ranges() {
    let c = WarpFormants {
        amount: -1.0,
        shift: 99.0,
        envelope: 1000.0,
    }
    .clamped()
    .unwrap();
    assert_eq!((c.amount, c.shift, c.envelope), (0.0, 24.0, 256.0));
    let nan = WarpFormants {
        envelope: f32::INFINITY,
        ..WarpFormants::DEFAULT
    };
    assert_eq!(nan.clamped(), None);
}

/// A project with one audio clip whose formants are `f`.
fn with_formants(f: WarpFormants) -> Project {
    let mut ids = IdGen::new(3);
    let mut p = Project::new(&mut ids, 1_000);
    let track: TrackId = ids.next(1_001);
    p.tracks.insert(
        track,
        Track {
            vca: None,
            freeze: None,
            id: track,
            kind: TrackKind::Audio,
            name: "t".into(),
            color: Color(0),
            order: OrderKey::between(None, None),
            parent: None,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::Auto,
            scale: Default::default(),
            mpe: None,
        },
    );
    let media: MediaId = ids.next(1_002);
    p.media.insert(
        media,
        MediaRef {
            id: media,
            name: "m".into(),
            file: "media/m.wav".into(),
            sample_rate: 48_000,
            channels: 2,
            frames: 48_000,
            hash: None,
            location: MediaLocation::Project,
        },
    );
    let clip: ClipId = ids.next(1_003);
    p.clips.insert(
        clip,
        Clip {
            lane: None,
            id: clip,
            track,
            start: Beats(0.0),
            name: String::new(),
            color: None,
            muted: false,
            length: Beats(4.0),
            offset: Beats::ZERO,
            looping: ClipLoop {
                enabled: false,
                start: Beats::ZERO,
                end: Beats(4.0),
            },
            content: ClipContent::Audio(AudioContent {
                media,
                gain: Decibels(0.0),
                transpose: 0.0,
                fade_in: Beats(0.0),
                fade_out: Beats(0.0),
                warp: WarpSettings {
                    mode: WarpMode::ComplexPlus,
                    formants: f,
                    ..WarpSettings::default()
                },
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                reversed: false,
            }),
        },
    );
    p
}

#[test]
fn documents_with_formants_out_of_range_are_refused() {
    let ok = with_formants(WarpFormants {
        amount: 0.5,
        shift: -24.0,
        envelope: 256.0,
    });
    ok.validate().unwrap();
    // What is accepted saves and loads back.
    let back = file::load(&file::save(&ok, "test").unwrap()).unwrap();
    assert_eq!(back.clips, ok.clips);
    for bad in [
        WarpFormants {
            shift: f32::INFINITY,
            ..WarpFormants::DEFAULT
        },
        WarpFormants {
            shift: 30.0,
            ..WarpFormants::DEFAULT
        },
        WarpFormants {
            amount: 1.5,
            ..WarpFormants::DEFAULT
        },
        WarpFormants {
            envelope: 1.0,
            ..WarpFormants::DEFAULT
        },
    ] {
        assert!(with_formants(bad).validate().is_err(), "{bad:?}");
    }
    // `1e39` overflows an f32 to infinity when the document is read: refused, not saved as
    // `null` (which would no longer load).
    let json = file::save(&ok, "test").unwrap().replace("-24.0", "-1e39");
    assert!(json.contains("-1e39"));
    assert!(file::load(&json).is_err());
}
