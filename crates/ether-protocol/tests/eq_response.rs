//! Test vectors of `ether_protocol::eq_response` for the UI mirror. The
//! TS implementation must reproduce every vector within 1e-6 dB. Regenerate with
//! `UPDATE_EQ_VECTORS=1 cargo test -p ether-protocol --test eq_response` (never by hand).
//!
//! The check is "same inputs, `db` within [`DB_TOLERANCE`]" rather than bit-equality:
//! `magnitude_db` goes through `tan`, `powf` and `log10`, which are not correctly rounded and
//! differ by an ULP or so between platform libms (Apple libm vs glibc). The tolerance is the
//! same 1e-6 dB contract the TS mirror is held to. `graphic_eq_vectors.json` holds the graphic
//! EQ's bands (`graphic_eq_db`), same contract.

use ether_protocol::eq_response::{
    EqShape, GRAPHIC_EQ_SLOPES, GraphicEqShape, graphic_eq_db, magnitude_db,
};

/// Max allowed |stored - computed| in dB (the TS mirror's contract, far below audibility).
const DB_TOLERANCE: f64 = 1e-6;

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
            (16_000.0, 9.0, 2.0),
            (19_000.0, -6.0, 0.3),
        ] {
            for sr in [44_100.0, 48_000.0] {
                for at in [
                    20.0, 60.0, 250.0, 1000.0, 3000.0, 10_000.0, 15_000.0, 20_000.0,
                ] {
                    let db = magnitude_db(shape, freq, gain, q, at, sr);
                    vectors.push(serde_json::json!({
                        "shape": shape, "freq": freq, "gain_db": gain, "q": q,
                        "at_hz": at, "sample_rate": sr, "db": db,
                    }));
                }
            }
        }
    }
    check(vectors, "eq_response_vectors.json");
}

/// The graphic EQ's bands: every shape, every slope of the shapes that have one.
#[test]
fn graphic_vectors_are_current() {
    let mut vectors = Vec::new();
    for shape in GraphicEqShape::ALL {
        for (freq, gain, q) in [
            (80.0, 6.0, 0.707),
            (1000.0, -12.0, 2.0),
            (12_000.0, 9.0, 0.5),
        ] {
            for slope in 0..GRAPHIC_EQ_SLOPES.len() {
                if slope != 1 && !shape.uses_slope() {
                    continue;
                }
                for sr in [44_100.0, 48_000.0] {
                    for at in [20.0, 100.0, 700.0, 1000.0, 5000.0, 15_000.0, 20_000.0] {
                        let db = graphic_eq_db(shape, freq, gain, q, slope, at, sr);
                        vectors.push(serde_json::json!({
                            "shape": shape, "freq": freq, "gain_db": gain, "q": q,
                            "slope": slope, "at_hz": at, "sample_rate": sr, "db": db,
                        }));
                    }
                }
            }
        }
    }
    check(vectors, "graphic_eq_vectors.json");
}

fn check(vectors: Vec<serde_json::Value>, file: &str) {
    let expected = serde_json::Value::Array(vectors);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(file);
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
    let (found, expected) = (found.as_array().unwrap(), expected.as_array().unwrap());
    assert_eq!(
        found.len(),
        expected.len(),
        "stale: run with UPDATE_EQ_VECTORS=1"
    );
    for (f, e) in found.iter().zip(expected) {
        let strip = |v: &serde_json::Value| {
            let mut v = v.clone();
            v.as_object_mut().unwrap().remove("db");
            v
        };
        assert_eq!(strip(f), strip(e), "stale: run with UPDATE_EQ_VECTORS=1");
        let (fd, ed) = (f["db"].as_f64().unwrap(), e["db"].as_f64().unwrap());
        assert!(
            (fd - ed).abs() <= DB_TOLERANCE,
            "{e}: stored {fd} dB, computed {ed} dB (stale: run with UPDATE_EQ_VECTORS=1)"
        );
    }
}
