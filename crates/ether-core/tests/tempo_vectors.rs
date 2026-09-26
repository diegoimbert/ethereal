//! Runs the shared tempo vectors (`crates/ether-model/tests/tempo_vectors.json`, generated
//! from `ether_model::TempoMap`) through the engine's `TempoMapRt`, so the engine and the
//! document model (and the UI, which uses the same file) agree.

use ether_core::protocol::model::{TempoCurve, TimeSignature};
use ether_core::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};
use serde_json::Value;

const VECTORS: &str = include_str!("../../ether-model/tests/tempo_vectors.json");

fn pairs(case: &Value, key: &str) -> Vec<(f64, Value)> {
    case[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p[0].as_f64().unwrap(), p[1].clone()))
        .collect()
}

#[test]
fn engine_tempo_map_matches_shared_vectors() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let tol = doc["tolerance"].as_f64().unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let tempo: Vec<TempoPointDesc> = case["tempo"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| TempoPointDesc {
                beat: p["time"].as_f64().unwrap(),
                bpm: p["bpm"].as_f64().unwrap(),
                curve: serde_json::from_value::<TempoCurve>(p["curve"].clone()).unwrap(),
            })
            .collect();
        let sigs: Vec<TimeSignatureDesc> = case["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| TimeSignatureDesc {
                beat: s["time"].as_f64().unwrap(),
                signature: serde_json::from_value::<TimeSignature>(s["signature"].clone())
                    .unwrap(),
            })
            .collect();
        let map = TempoMapRt::compile(&tempo, &sigs);

        let check = |what: &str, input: f64, got: f64, want: f64| {
            let t = tol * want.abs().max(1.0);
            assert!(
                (got - want).abs() <= t,
                "{name}: {what}({input}) = {got}, expected {want}"
            );
        };
        for (b, s) in pairs(case, "beats_to_seconds") {
            check("beats_to_seconds", b, map.beats_to_seconds(b), s.as_f64().unwrap());
        }
        for (s, b) in pairs(case, "seconds_to_beats") {
            check("seconds_to_beats", s, map.seconds_to_beats(s), b.as_f64().unwrap());
        }
        for (b, bpm) in pairs(case, "bpm_at") {
            check("bpm_at", b, map.bpm_at(b), bpm.as_f64().unwrap());
        }
        // The RT map exposes the bar start (not the bar number): check beat-in-bar and
        // fraction, which depend on the bar start and the signature.
        for (b, bb) in pairs(case, "bar_beat") {
            let (sig, bar_start) = map.signature_at(b);
            let unit = 4.0 / sig.denominator as f64;
            let rel = (b - bar_start) / unit;
            let mut beat = (rel + 1e-6).floor();
            let mut fraction = rel - beat;
            if fraction < 0.0 {
                fraction = 0.0;
            }
            if fraction.abs() < 1e-6 {
                fraction = 0.0;
            }
            beat += 1.0;
            assert_eq!(
                beat as u64,
                bb["beat"].as_u64().unwrap(),
                "{name}: beat at {b}"
            );
            check("bar fraction", b, fraction, bb["fraction"].as_f64().unwrap());
        }
    }
}
