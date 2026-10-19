//! Runs the shared fade-law vectors (`ui/src/features/clip-editing/fadeVectors.json`, also
//! checked by the UI's `fades.test.ts`) through `ether_core::fades::fade_gain`, so the
//! engine and the UI's fade drawing agree.

use ether_core::fades::fade_gain;
use ether_core::protocol::model::FadeCurve;
use serde_json::Value;

const VECTORS: &str = include_str!("../../../ui/src/features/clip-editing/fadeVectors.json");

#[test]
fn fade_gain_matches_shared_vectors() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let tol = doc["tolerance"].as_f64().unwrap() as f32;
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 3);
    for case in cases {
        let curve: FadeCurve = serde_json::from_value(case["curve"].clone()).unwrap();
        for p in case["points"].as_array().unwrap() {
            let x = p[0].as_f64().unwrap() as f32;
            let want = p[1].as_f64().unwrap() as f32;
            let got = fade_gain(curve, x);
            assert!(
                (got - want).abs() <= tol,
                "{curve:?} at {x}: {got} != {want}"
            );
        }
    }
}
