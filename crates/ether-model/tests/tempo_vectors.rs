//! Shared tempo-map test vectors: `tests/tempo_vectors.json`, generated from
//! `ether_model::TempoMap` (the source of truth). `ether-core` and the UI check their tempo
//! math against the file.
//!
//! Regenerate after an intentional semantic change:
//! `ETHER_UPDATE_TEMPO_VECTORS=1 cargo test -p ether-model --test tempo_vectors`

use std::path::PathBuf;

use ether_model::*;
use serde_json::{Value, json};

const TOLERANCE: f64 = 1e-9;

type P = (f64, f64, TempoCurve);
type S = (f64, u8, u8);

fn cases() -> Vec<(&'static str, Vec<P>, Vec<S>)> {
    use TempoCurve::{Linear, Step};
    vec![
        ("constant_120", vec![(0.0, 120.0, Step)], vec![(0.0, 4, 4)]),
        (
            "steps",
            vec![(0.0, 120.0, Step), (8.0, 60.0, Step), (16.0, 140.0, Step)],
            vec![(0.0, 4, 4)],
        ),
        (
            "ramp_up",
            vec![(0.0, 60.0, Linear), (4.0, 120.0, Step)],
            vec![(0.0, 4, 4)],
        ),
        (
            // A trailing Linear point has no next point: constant.
            "ramp_down_then_trailing_linear",
            vec![
                (0.0, 140.0, Linear),
                (8.0, 70.0, Step),
                (12.0, 70.0, Linear),
            ],
            vec![(0.0, 3, 4)],
        ),
        (
            "mixed",
            vec![
                (0.0, 100.0, Step),
                (4.0, 100.0, Linear),
                (12.0, 180.0, Linear),
                (20.0, 90.0, Step),
                (32.0, 128.0, Step),
            ],
            vec![(0.0, 4, 4), (8.0, 6, 8), (14.0, 5, 4)],
        ),
        (
            "linear_without_bpm_change",
            vec![(0.0, 120.0, Linear), (4.0, 120.0, Step)],
            vec![(0.0, 4, 4)],
        ),
        (
            // Linear into a point at the same time (len 0 <= EPS): constant, no ramp.
            "zero_length_ramp",
            vec![
                (0.0, 120.0, Linear),
                (4.0, 90.0, Linear),
                (4.0, 150.0, Step),
            ],
            vec![(0.0, 4, 4)],
        ),
        (
            "fractional_times",
            vec![
                (0.0, 97.5, Linear),
                (2.75, 133.25, Step),
                (7.125, 61.0, Linear),
                (9.5, 200.0, Step),
            ],
            vec![(0.0, 7, 8)],
        ),
        (
            // Off-bar signature change at beat 6 (mid bar 2 of 4/4): the partial bar counts
            // as one bar, so bar 3 starts at beat 6.
            "off_bar_signature_changes",
            vec![(0.0, 120.0, Step)],
            vec![
                (0.0, 4, 4),
                (6.0, 3, 4),
                (12.0, 6, 8),
                (15.0, 7, 8),
                (18.5, 2, 2),
            ],
        ),
    ]
}

const BEATS: &[f64] = &[
    -9.0,
    -4.0,
    -0.5,
    0.0,
    0.25,
    1.0,
    2.0,
    2.75,
    3.5,
    3.999_999_9,
    4.0,
    5.0,
    6.0,
    6.3,
    7.125,
    7.999_999_5,
    8.0,
    8.5,
    9.5,
    10.0,
    11.75,
    12.0,
    13.0,
    14.0,
    15.0,
    16.0,
    17.5,
    18.5,
    20.0,
    21.0,
    25.0,
    32.0,
    40.0,
    100.0,
];

fn build(tempo: &[P], sigs: &[S]) -> TempoMap {
    TempoMap {
        tempo: tempo
            .iter()
            .map(|&(t, bpm, curve)| TempoPoint {
                id: TempoPointId::NIL,
                time: Beats(t),
                bpm,
                curve,
            })
            .collect(),
        signatures: sigs
            .iter()
            .map(|&(t, n, d)| TimeSignaturePoint {
                id: TimeSignatureId::NIL,
                time: Beats(t),
                signature: TimeSignature {
                    numerator: n,
                    denominator: d,
                },
            })
            .collect(),
    }
}

fn generate() -> Value {
    let cases: Vec<Value> = cases()
        .into_iter()
        .map(|(name, tempo, sigs)| {
            let m = build(&tempo, &sigs);
            let b2s: Vec<(f64, f64)> = BEATS
                .iter()
                .map(|&b| (b, m.beats_to_seconds(Beats(b)).0))
                .collect();
            json!({
                "name": name,
                "tempo": tempo.iter().map(|&(t, bpm, c)| json!({"time": t, "bpm": bpm, "curve": c})).collect::<Vec<_>>(),
                "signatures": sigs.iter().map(|&(t, n, d)| json!({"time": t, "signature": {"numerator": n, "denominator": d}})).collect::<Vec<_>>(),
                "beats_to_seconds": b2s.iter().map(|&(b, s)| json!([b, s])).collect::<Vec<_>>(),
                "seconds_to_beats": b2s.iter().map(|&(_, s)| json!([s, m.seconds_to_beats(Seconds(s)).0])).collect::<Vec<_>>(),
                "bpm_at": BEATS.iter().map(|&b| json!([b, m.bpm_at(Beats(b))])).collect::<Vec<_>>(),
                "bar_beat": BEATS.iter().map(|&b| json!([b, m.bar_beat(Beats(b))])).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "description": "Shared tempo-map test vectors, generated from ether_model::TempoMap by \
            crates/ether-model/tests/tempo_vectors.rs (do not edit by hand). Seconds are measured \
            from beat 0. Step = constant until the next point; Linear = BPM ramps linearly over \
            beats to the next point's BPM (only when the segment is longer than 1e-6 beats); the \
            first tempo holds before the first point and the last one after it. bar_beat: bar 1 \
            beat 1 = beat 0, beats in denominator units, a partial bar before an off-bar \
            signature change counts as one bar.",
        "tolerance": TOLERANCE,
        "cases": cases,
    })
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tempo_vectors.json")
}

/// Structural equality with numeric tolerance (libm results may differ in the last ulp
/// across platforms).
fn approx_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            (x - y).abs() <= 1e-12 * x.abs().max(1.0)
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| approx_eq(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| approx_eq(v, w)))
        }
        _ => a == b,
    }
}

#[test]
fn tempo_vectors_file_is_up_to_date() {
    let generated = generate();
    if std::env::var_os("ETHER_UPDATE_TEMPO_VECTORS").is_some() {
        let text = serde_json::to_string_pretty(&generated).unwrap() + "\n";
        std::fs::write(path(), text).unwrap();
        return;
    }
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(path()).expect("tests/tempo_vectors.json is missing"),
    )
    .unwrap();
    assert!(
        approx_eq(&generated, &on_disk),
        "tests/tempo_vectors.json is stale: run \
         ETHER_UPDATE_TEMPO_VECTORS=1 cargo test -p ether-model --test tempo_vectors"
    );
}

/// Independent closed-form checks of a few vectors (so the generator isn't only checked
/// against itself).
#[test]
fn vectors_match_closed_forms() {
    let v = generate();
    let case = |name: &str| {
        v["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()
            .clone()
    };
    let lookup = |c: &Value, table: &str, x: f64| -> f64 {
        c[table]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p[0].as_f64().unwrap() == x)
            .unwrap()[1]
            .as_f64()
            .unwrap()
    };
    let close = |a: f64, b: f64| (a - b).abs() < TOLERANCE;

    let steps = case("steps");
    assert!(close(lookup(&steps, "beats_to_seconds", 10.0), 4.0 + 2.0));
    assert!(close(
        lookup(&steps, "beats_to_seconds", 20.0),
        4.0 + 8.0 + 4.0 * 60.0 / 140.0
    ));
    assert!(close(lookup(&steps, "beats_to_seconds", -4.0), -2.0));

    // ∫0..4 60/(60+15x) dx = 4 ln 2; then 120 bpm.
    let ramp = case("ramp_up");
    assert!(close(
        lookup(&ramp, "beats_to_seconds", 4.0),
        4.0 * 2f64.ln()
    ));
    assert!(close(
        lookup(&ramp, "beats_to_seconds", 5.0),
        4.0 * 2f64.ln() + 0.5
    ));
    assert!(close(lookup(&ramp, "bpm_at", 2.0), 90.0));

    let zero = case("zero_length_ramp");
    // 120→90 ramp over 4 beats (60·4/-30·ln(90/120)), the 0-length segment adds nothing,
    // then 150 bpm.
    assert!(close(
        lookup(&zero, "beats_to_seconds", 5.0),
        -8.0 * 0.75f64.ln() + 60.0 / 150.0
    ));
    assert!(close(lookup(&zero, "bpm_at", 4.0), 150.0));

    for c in v["cases"].as_array().unwrap() {
        let tol = v["tolerance"].as_f64().unwrap();
        // seconds_to_beats inverts beats_to_seconds (pairs are index-aligned).
        let b2s = c["beats_to_seconds"].as_array().unwrap();
        let s2b = c["seconds_to_beats"].as_array().unwrap();
        for (x, y) in b2s.iter().zip(s2b) {
            let b = x[0].as_f64().unwrap();
            let back = y[1].as_f64().unwrap();
            assert!((back - b).abs() < tol, "{} {b} -> {back}", c["name"]);
        }
    }

    let off = case("off_bar_signature_changes");
    let bb = |b: f64| {
        let e = off["bar_beat"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p[0].as_f64().unwrap() == b)
            .unwrap()[1]
            .clone();
        (e["bar"].as_i64().unwrap(), e["beat"].as_u64().unwrap())
    };
    assert_eq!(bb(0.0), (1, 1));
    assert_eq!(bb(5.0), (2, 2));
    assert_eq!(bb(6.0), (3, 1)); // partial bar 2 (2 beats) counts as one bar
    assert_eq!(bb(9.5), (4, 1));
    assert_eq!(bb(12.0), (5, 1)); // 6/8
    assert_eq!(bb(15.0), (6, 1)); // 7/8
    assert_eq!(bb(18.5), (7, 1)); // 2/2
    assert_eq!(bb(21.0), (7, 2)); // half-note beats
    assert_eq!(bb(3.999_999_9), (2, 1));
    assert_eq!(bb(-4.0), (0, 1));
}
