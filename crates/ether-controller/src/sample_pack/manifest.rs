//! The pack manifest (docs/SAMPLES.md "Manifest"): its shape, the signature check and the
//! structural validation. The same envelope as the plugin catalog
//! (`{"manifest": "<json text>", "signature": "<base64>"}`), signed with the same Ed25519
//! key, but over [`SIGNING_CONTEXT`] + the manifest text, so neither signature can stand in
//! for the other.

use std::collections::BTreeSet;

use base64::Engine as _;
use ether_core::protocol::samples::{SampleCategory, SampleLicense};
use serde::{Deserialize, Serialize};

/// The manifest schema this version reads.
pub const SCHEMA: u32 = 1;

/// Prefix of the signed bytes (domain separation from the plugin catalog's manifests).
pub const SIGNING_CONTEXT: &[u8] = b"ethereal sample pack v1\n";

/// The production signing key (raw Ed25519, base64): the plugin catalog's
/// (`PLUGIN_CATALOG_SIGNING_KEY` in sops; `ether_plugin_store::CATALOG_PUBLIC_KEY`).
pub const PUBLIC_KEY: &str = "zulFanzJ9EYe322beZnZE3sx6dx65ekPxcL7qLmD67E=";

/// The seed of the dev fixture key (public on purpose: tests and the `sample_pack` example's
/// `dev` pack sign with it). Accepted only for manifests served from a loopback address.
pub const DEV_SEED: [u8; 32] = *b"ethereal-dev-sample-pack-seed-v1";

/// A manifest bigger than this is refused before parsing.
pub const MAX_MANIFEST_BYTES: u64 = 4 << 20;
/// A file bigger than this is refused (the manifest validation and the download cap).
pub const MAX_FILE_BYTES: u64 = 64 << 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// The pack id (`default`): no `/`.
    pub id: String,
    /// Monotonic: a manifest older than the cached one is refused (rollback).
    pub version: u64,
    pub name: String,
    #[serde(default)]
    pub generated_at: String,
    pub files: Vec<PackFile>,
    #[serde(default)]
    pub kits: Vec<Kit>,
    #[serde(default)]
    pub instruments: Vec<Instrument>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackFile {
    /// Stable id within the pack (`drums/909/kick`): projects reference it.
    pub id: String,
    pub name: String,
    /// Relative to the manifest's URL: `files/<sha256>.<wav|flac>` (content-addressed).
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub category: SampleCategory,
    pub license: SampleLicense,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub key: Option<String>,
    /// Optional precomputed preview (a shorter excerpt of a long loop): same rules as the
    /// file. Not used by this version (it computes peaks and previews itself).
    #[serde(default)]
    pub preview: Option<Preview>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kit {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub pads: Vec<KitPad>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KitPad {
    /// MIDI note of the pad (36 = C1, the first pad).
    pub note: u8,
    /// A file id of this pack.
    pub sample: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub choke: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub zones: Vec<Zone>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub sample: String,
    pub root: u8,
    #[serde(default)]
    pub lo_key: u8,
    #[serde(default = "max_midi")]
    pub hi_key: u8,
    #[serde(default)]
    pub lo_vel: u8,
    #[serde(default = "max_midi")]
    pub hi_vel: u8,
}

fn max_midi() -> u8 {
    127
}

/// The wire shape of the manifest URL.
#[derive(Debug, Deserialize, Serialize)]
pub struct Envelope {
    pub manifest: String,
    pub signature: String,
}

/// Decode a raw Ed25519 public key (base64).
pub fn public_key(b64: &str) -> Option<[u8; 32]> {
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .ok()?
        .try_into()
        .ok()
}

/// The dev fixture key's public half.
pub fn dev_public_key() -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(&DEV_SEED)
        .verifying_key()
        .to_bytes()
}

/// Whether `url` is served from this machine (`http(s)://localhost|127.0.0.1|[::1]`): the
/// only sources that may present the dev key.
pub fn is_loopback(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    let Some(rest) = rest else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// The keys a manifest from `url` may be signed with.
pub fn keys_for(url: &str) -> Vec<[u8; 32]> {
    let mut keys: Vec<[u8; 32]> = public_key(PUBLIC_KEY).into_iter().collect();
    if is_loopback(url) {
        keys.push(dev_public_key());
    }
    keys
}

/// Sign `manifest` (tests and the dev fixture; the owner's script signs with Node).
pub fn sign(manifest: &str, seed: &[u8; 32]) -> String {
    use ed25519_dalek::Signer as _;
    let key = ed25519_dalek::SigningKey::from_bytes(seed);
    let mut msg = SIGNING_CONTEXT.to_vec();
    msg.extend_from_slice(manifest.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(key.sign(&msg).to_bytes())
}

/// Check an envelope's signature against `keys`, then parse and validate the manifest.
pub fn verify(envelope: &[u8], keys: &[[u8; 32]]) -> Result<Manifest, String> {
    if envelope.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("the manifest is too big".into());
    }
    let env: Envelope =
        serde_json::from_slice(envelope).map_err(|e| format!("malformed envelope: {e}"))?;
    let sig = base64::engine::general_purpose::STANDARD
        .decode(env.signature.trim())
        .ok()
        .and_then(|b| ed25519_dalek::Signature::from_slice(&b).ok())
        .ok_or("malformed signature")?;
    let mut msg = SIGNING_CONTEXT.to_vec();
    msg.extend_from_slice(env.manifest.as_bytes());
    let signed = keys.iter().any(|k| {
        ed25519_dalek::VerifyingKey::from_bytes(k)
            .is_ok_and(|k| k.verify_strict(&msg, &sig).is_ok())
    });
    if !signed {
        return Err("bad signature".into());
    }
    let m: Manifest =
        serde_json::from_str(&env.manifest).map_err(|e| format!("malformed manifest: {e}"))?;
    if m.schema != SCHEMA {
        return Err(format!(
            "pack schema {} is not supported by this version (update ethereal)",
            m.schema
        ));
    }
    validate(&m)?;
    Ok(m)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A content-addressed file path: `files/<sha256>.<ext>` with the given hash.
fn check_path(path: &str, sha256: &str, exts: &[&str]) -> Result<(), String> {
    let ok = path
        .strip_prefix("files/")
        .and_then(|rest| rest.split_once('.'))
        .is_some_and(|(hash, ext)| hash == sha256 && exts.contains(&ext));
    if ok {
        Ok(())
    } else {
        Err(format!("bad file path \"{path}\""))
    }
}

fn check_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if ok {
        Ok(())
    } else {
        Err(format!("bad id \"{id}\""))
    }
}

/// Structural checks (the publish script runs the same rules before signing).
pub fn validate(m: &Manifest) -> Result<(), String> {
    check_id(&m.id)?;
    if m.id.contains('/') || m.id == ether_core::protocol::samples::BUILTIN_PACK {
        return Err(format!("bad pack id \"{}\"", m.id));
    }
    let mut ids = BTreeSet::new();
    for f in &m.files {
        check_id(&f.id)?;
        if !ids.insert(f.id.as_str()) {
            return Err(format!("duplicate file id \"{}\"", f.id));
        }
        if !is_hex64(&f.sha256) {
            return Err(format!("bad sha256 for \"{}\"", f.id));
        }
        check_path(&f.path, &f.sha256, &["wav", "flac"])?;
        if f.size == 0 || f.size > MAX_FILE_BYTES {
            return Err(format!("bad size for \"{}\"", f.id));
        }
        if f.sample_rate == 0 || !(1..=2).contains(&f.channels) || f.frames == 0 {
            return Err(format!("bad audio format for \"{}\"", f.id));
        }
        if f.bpm.is_some_and(|b| !(20.0..=999.0).contains(&b)) {
            return Err(format!("bad bpm for \"{}\"", f.id));
        }
        if let Some(p) = &f.preview {
            if !is_hex64(&p.sha256) || p.size == 0 || p.size > MAX_FILE_BYTES {
                return Err(format!("bad preview for \"{}\"", f.id));
            }
            check_path(&p.path, &p.sha256, &["mp3", "flac", "wav"])?;
        }
    }
    let mut groups = BTreeSet::new();
    for k in &m.kits {
        check_id(&k.id)?;
        if !groups.insert(("kit", k.id.as_str())) {
            return Err(format!("duplicate kit id \"{}\"", k.id));
        }
        let mut notes = BTreeSet::new();
        for p in &k.pads {
            if p.note > 127 || !notes.insert(p.note) || !ids.contains(p.sample.as_str()) {
                return Err(format!("bad pad {} in kit \"{}\"", p.note, k.id));
            }
        }
    }
    for i in &m.instruments {
        check_id(&i.id)?;
        if !groups.insert(("instrument", i.id.as_str())) || i.zones.is_empty() {
            return Err(format!("bad instrument \"{}\"", i.id));
        }
        for z in &i.zones {
            let ok = ids.contains(z.sample.as_str())
                && z.root <= 127
                && z.lo_key <= z.hi_key
                && z.hi_key <= 127
                && z.lo_vel <= z.hi_vel
                && z.hi_vel <= 127;
            if !ok {
                return Err(format!("bad zone in instrument \"{}\"", i.id));
            }
        }
    }
    Ok(())
}

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The URL of `path` (relative) next to the manifest at `manifest_url`.
pub fn file_url(manifest_url: &str, path: &str) -> String {
    let base = manifest_url
        .split(['?', '#'])
        .next()
        .unwrap_or(manifest_url);
    match base.rfind('/') {
        Some(i) => format!("{}/{path}", &base[..i]),
        None => path.to_string(),
    }
}
