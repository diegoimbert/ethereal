use super::cache::Cache;
use super::fixture::{dev_pack, envelope};
use super::manifest::{self, DEV_SEED, Envelope, PUBLIC_KEY};
use crate::memory::MemoryLibrary;

const LOCAL: &str = "http://127.0.0.1:4100/packs/default/manifest.json";

/// The publish script (Node, `scripts/samples/publish.test.mjs`) signs the same bytes: Ed25519
/// is deterministic, so both sides pin the same signature and dev public key.
#[test]
fn signatures_match_the_publish_script() {
    use base64::Engine as _;
    assert_eq!(
        manifest::sign("{\"x\":1}", &DEV_SEED),
        "MBl/0AGdIHm9sOIT/09cdFVv2vZCYVZ5gddzEU+WFfdL7BR2D3UJMPO8UcI9Zuvb7HP4RDrmQnN1V6OiVwdJBA=="
    );
    assert_eq!(
        base64::engine::general_purpose::STANDARD.encode(manifest::dev_public_key()),
        "iDAO7XZzV64vvHJr6a+cg3BInc3ph4PXOmxfWNznR80="
    );
}

#[test]
fn the_dev_pack_verifies_with_the_dev_key_from_loopback_only() {
    let pack = dev_pack(1, 1.0);
    let m = manifest::verify(&pack.envelope, &manifest::keys_for(LOCAL)).unwrap();
    assert_eq!(m, pack.manifest);
    let remote = "https://samples.example.com/manifest.json";
    assert_eq!(
        manifest::verify(&pack.envelope, &manifest::keys_for(remote)).unwrap_err(),
        "bad signature"
    );
}

#[test]
fn loopback_hosts() {
    for url in [
        "http://localhost/x",
        "http://127.0.0.1:9/x",
        "https://[::1]:8443/x",
    ] {
        assert!(manifest::is_loopback(url), "{url}");
    }
    for url in [
        "http://localhost.evil.com/x",
        "http://127.0.0.2/x",
        "https://evil.com/?h=localhost",
        "file:///etc",
        "http://user@evil.com/x",
    ] {
        assert!(!manifest::is_loopback(url), "{url}");
    }
}

#[test]
fn the_production_key_is_the_catalog_key() {
    assert!(manifest::public_key(PUBLIC_KEY).is_some());
    assert_ne!(
        manifest::public_key(PUBLIC_KEY),
        Some(manifest::dev_public_key())
    );
}

#[test]
fn a_tampered_manifest_or_signature_is_refused() {
    let pack = dev_pack(1, 1.0);
    let keys = manifest::keys_for(LOCAL);
    let mut env: Envelope = serde_json::from_slice(&pack.envelope).unwrap();
    env.manifest = env.manifest.replace("dev kit", "evil kit");
    let tampered = serde_json::to_vec(&env).unwrap();
    assert_eq!(
        manifest::verify(&tampered, &keys).unwrap_err(),
        "bad signature"
    );

    // A plugin-catalog style signature (no context prefix) doesn't pass.
    use base64::Engine as _;
    use ed25519_dalek::Signer as _;
    let text = serde_json::to_string(&pack.manifest).unwrap();
    let raw = ed25519_dalek::SigningKey::from_bytes(&DEV_SEED).sign(text.as_bytes());
    let env = Envelope {
        manifest: text,
        signature: base64::engine::general_purpose::STANDARD.encode(raw.to_bytes()),
    };
    let unprefixed = serde_json::to_vec(&env).unwrap();
    assert_eq!(
        manifest::verify(&unprefixed, &keys).unwrap_err(),
        "bad signature"
    );
    assert!(manifest::verify(b"not json", &keys).is_err());
}

#[test]
fn structural_errors_are_refused_even_when_signed() {
    let keys = manifest::keys_for(LOCAL);
    let base = dev_pack(1, 1.0).manifest;
    type Edit = Box<dyn Fn(&mut manifest::Manifest)>;
    let cases: Vec<(&str, Edit)> = vec![
        ("schema", Box::new(|m| m.schema = 2)),
        (
            "path",
            Box::new(|m| m.files[0].path = "../../etc/passwd".into()),
        ),
        (
            "hash path",
            Box::new(|m| m.files[0].path = format!("files/{}.wav", "0".repeat(64))),
        ),
        ("dup", Box::new(|m| m.files[1].id = m.files[0].id.clone())),
        (
            "pad",
            Box::new(|m| m.kits[0].pads[0].sample = "nope".into()),
        ),
        ("zone", Box::new(|m| m.instruments[0].zones[0].hi_key = 200)),
        ("pack id", Box::new(|m| m.id = "builtin".into())),
        ("id", Box::new(|m| m.files[0].id = "a/../b".into())),
        ("size", Box::new(|m| m.files[0].size = 0)),
    ];
    for (name, edit) in cases {
        let mut m = base.clone();
        edit(&mut m);
        assert!(manifest::verify(&envelope(&m), &keys).is_err(), "{name}");
    }
}

#[test]
fn file_urls_sit_next_to_the_manifest() {
    assert_eq!(
        manifest::file_url("https://a.b/packs/default/manifest.json?v=2", "files/x.wav"),
        "https://a.b/packs/default/files/x.wav"
    );
}

#[test]
fn garbage_collection_keeps_current_and_pinned_files() {
    let mut lib = MemoryLibrary::new();
    let cache = Cache::for_url(LOCAL);
    let v1 = dev_pack(1, 1.0);
    let v2 = dev_pack(2, 2.0);
    for (f, (_, bytes)) in v1.manifest.files.iter().zip(&v1.files) {
        cache.write(&mut lib, f, bytes).unwrap();
    }
    // v2 changed the instrument file only; a project uses the old one... and one more file
    // dropped from v2 that nobody pinned.
    let old = &v1.manifest.files[4];
    assert_ne!(old.sha256, v2.manifest.files[4].sha256);
    assert!(super::cache::pin(&mut lib, &old.sha256));
    assert!(!super::cache::pin(&mut lib, &old.sha256));
    let mut v3 = v2.manifest.clone();
    v3.files.remove(0);
    v3.kits[0].pads.remove(0);
    let removed = cache.collect_garbage(&mut lib, &v3);
    assert_eq!(removed, 1, "the dropped, unpinned kick");
    let present = super::cache::present(&mut lib);
    assert!(present.contains(&old.sha256));
    assert!(!present.contains(&v1.manifest.files[0].sha256));
    assert_eq!(present.len(), 4);

    // Files another source's cached manifest lists survive this source's updates.
    let kick = &v1.manifest.files[0];
    cache.write(&mut lib, kick, &v1.files[0].1).unwrap();
    let other = Cache::for_url("http://localhost:9/other/manifest.json");
    other.store_manifest(&mut lib, &v1.envelope).unwrap();
    assert_eq!(cache.collect_garbage(&mut lib, &v3), 0);
    assert!(super::cache::present(&mut lib).contains(&kick.sha256));
}

#[test]
fn a_damaged_cached_file_is_dropped_on_read() {
    let mut lib = MemoryLibrary::new();
    let cache = Cache::for_url(LOCAL);
    let pack = dev_pack(1, 1.0);
    let f = &pack.manifest.files[0];
    cache.write(&mut lib, f, b"garbage").unwrap();
    assert!(cache.read(&mut lib, f).is_err());
    assert!(super::cache::present(&mut lib).is_empty());
    cache.write(&mut lib, f, &pack.files[0].1).unwrap();
    assert_eq!(cache.read(&mut lib, f).unwrap(), pack.files[0].1);
}
