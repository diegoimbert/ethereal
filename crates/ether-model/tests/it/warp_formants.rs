//! `WarpMode::ComplexPlus` and Complex+'s formant settings in documents: defaults are not
//! written (documents without them keep re-saving byte for byte), other values round-trip.

use ether_model::{WarpFormants, WarpMode, WarpSettings};

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
