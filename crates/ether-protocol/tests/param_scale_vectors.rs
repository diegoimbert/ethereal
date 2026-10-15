//! Shared param-scale vectors (`param_scale_vectors.json`), also checked by the UI
//! (`ui/src/app/contracts.test.ts`) and the controller (`tests/contract_vectors.rs`).

use ether_protocol::devices::{ParamScale, scale_to_normalized, scale_to_plain};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("param_scale_vectors.json")).expect("valid JSON")
}

#[test]
fn scale_to_plain_matches_the_shared_vectors() {
    let v = vectors();
    let tol = v["tolerance"].as_f64().unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.len() > 20);
    for c in cases {
        let scale: ParamScale = serde_json::from_value(c["scale"].clone()).unwrap();
        let (min, max) = (c["min"].as_f64().unwrap(), c["max"].as_f64().unwrap());
        let n = c["normalized"].as_f64().unwrap();
        let plain = c["plain"].as_f64().unwrap();
        let got = scale_to_plain(scale, min, max, n);
        assert!((got - plain).abs() <= tol, "{c}: got {got}");
        // Round trip (except where the curve is flat at the bottom).
        if n > 0.0 && plain > min {
            let back = scale_to_normalized(scale, min, max, plain);
            assert!((back - n).abs() <= 1e-6, "{c}: back {back}");
        }
    }
}
