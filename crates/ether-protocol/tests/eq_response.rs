//! Test vectors of `ether_protocol::eq_response` for the UI mirror (`graphical-eq`). The
//! TS implementation must reproduce every vector within 1e-6 dB. Regenerate with
//! `UPDATE_EQ_VECTORS=1 cargo test -p ether-protocol --test eq_response` (never by hand).

use ether_protocol::eq_response::{EqShape, magnitude_db};

const SHAPES: [EqShape; 9] = [
    EqShape::LowCut,
    EqShape::LowShelf,
    EqShape::Bell,
    EqShape::Notch,
    EqShape::HighShelf,
    EqShape::HighCut,
    EqShape::LowCut24,
    EqShape::HighCut24,
    EqShape::BandPass,
];

#[test]
fn vectors_are_current() {
    let mut vectors = Vec::new();
    for shape in SHAPES {
        for (freq, gain, q) in [
            (100.0, 6.0, 0.707),
            (1000.0, -12.0, 2.0),
            (8000.0, 3.0, 0.5),
        ] {
            for sr in [44_100.0, 48_000.0] {
                for at in [20.0, 60.0, 250.0, 1000.0, 3000.0, 10_000.0, 20_000.0] {
                    let db = magnitude_db(shape, freq, gain, q, at, sr);
                    vectors.push(serde_json::json!({
                        "shape": shape, "freq": freq, "gain_db": gain, "q": q,
                        "at_hz": at, "sample_rate": sr, "db": db,
                    }));
                }
            }
        }
    }
    let expected = serde_json::Value::Array(vectors);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/eq_response_vectors.json");
    if std::env::var_os("UPDATE_EQ_VECTORS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&expected).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let found: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("vectors file")).unwrap();
    assert_eq!(found, expected, "stale: run with UPDATE_EQ_VECTORS=1");
}
