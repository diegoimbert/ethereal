//! Secrets of a shared project and what is derived from them (docs/SHARING.md §4.2-§4.3).
//! Frozen: the derivations below are pinned by the test vectors at the end of this file
//! (also written into docs/SHARING.md §4.2).
//!
//! - `K`: the 16 secret bytes of a link or member key ([`LinkKey`], base64url decoded).
//! - door = HMAC-SHA256(K, `"ethereal/share/v1/door"` ‖ room)\[..16\], base64url (22
//!   chars). The signaling service stores lowercase hex SHA-256 of the door *string*
//!   ([`door_hash`]); a joiner presents the door itself.
//! - join proof = HMAC-SHA256(K, `"ethereal/share/v1/join"` ‖ fp_joiner ‖ fp_host), base64url
//!   (43 chars); host proof = HMAC-SHA256(K, `"ethereal/share/v1/host"` ‖ fp_host ‖
//!   fp_joiner). `fp_*` are the DTLS fingerprints as in SDP (`"sha-256 AB:CD:..."`), so a
//!   signaling service that swaps fingerprints cannot relay a valid proof.
//! - Strings are concatenated as UTF-8 bytes, without separators (the labels are fixed and
//!   fingerprints have a fixed shape).
//!
//! Random ids and keys come from the OS (`getrandom`) natively and from
//! `crypto.getRandomValues` in the browser (the controller Worker).

use base64::Engine;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use super::invite::{ID_CHARS, KEY_V1, LinkKey, valid_id};

type HmacSha256 = Hmac<Sha256>;

pub const DOOR_LABEL: &str = "ethereal/share/v1/door";
pub const JOIN_LABEL: &str = "ethereal/share/v1/join";
pub const HOST_LABEL: &str = "ethereal/share/v1/host";
/// Length of a host token (32 bytes, base64url without padding).
pub const HOST_TOKEN_CHARS: usize = 43;

/// Lenient decoding (non-zero trailing bits accepted): a pasted key is valid by its
/// alphabet and length alone ([`valid_id`]), like the TS parser.
const DECODE: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

/// base64url without padding.
pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    DECODE.decode(s).ok()
}

/// `N` random bytes from the platform's secure generator. `Err` when there is none (never
/// fall back to a weak source: these are secrets).
pub fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut buf = [0u8; N];
    fill_random(&mut buf)?;
    Ok(buf)
}

#[cfg(not(target_arch = "wasm32"))]
fn fill_random(buf: &mut [u8]) -> Result<(), String> {
    getrandom::fill(buf).map_err(|e| format!("no secure random source: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn fill_random(buf: &mut [u8]) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let err = |_| "no secure random source (crypto.getRandomValues)".to_string();
    let crypto = js_sys::Reflect::get(&js_sys::global(), &"crypto".into()).map_err(err)?;
    let get: js_sys::Function = js_sys::Reflect::get(&crypto, &"getRandomValues".into())
        .map_err(err)?
        .dyn_into()
        .map_err(|_| err(wasm_bindgen::JsValue::NULL))?;
    let arr = js_sys::Uint8Array::new_with_length(buf.len() as u32);
    get.call1(&crypto, &arr).map_err(err)?;
    arr.copy_to(buf);
    Ok(())
}

/// A new room id or member id (16 random bytes, 22 chars).
pub fn new_id() -> Result<String, String> {
    Ok(b64url(&random_bytes::<16>()?))
}

/// A new v1 link or member key.
pub fn new_key() -> Result<LinkKey, String> {
    Ok(LinkKey {
        version: KEY_V1,
        secret: new_id()?,
    })
}

/// A new host token (32 random bytes, 43 chars).
pub fn new_host_token() -> Result<String, String> {
    Ok(b64url(&random_bytes::<32>()?))
}

/// Parse a key as stored in `share.json` (`LinkKey` text: version + secret).
pub fn parse_key(text: &str) -> Option<LinkKey> {
    let mut chars = text.chars();
    let version = chars.next()?;
    let secret = chars.as_str();
    (version == KEY_V1 && valid_id(secret)).then(|| LinkKey {
        version,
        secret: secret.to_string(),
    })
}

/// `K`: the key's secret bytes (`None`: not a v1 key).
fn key_bytes(key: &LinkKey) -> Option<Vec<u8>> {
    if key.version != KEY_V1 || key.secret.len() != ID_CHARS {
        return None;
    }
    b64url_decode(&key.secret).filter(|b| b.len() == 16)
}

fn mac(key: &LinkKey, parts: &[&str]) -> Option<HmacSha256> {
    let k = key_bytes(key)?;
    let mut m = HmacSha256::new_from_slice(&k).expect("HMAC takes any key length");
    for p in parts {
        m.update(p.as_bytes());
    }
    Some(m)
}

/// The door of `key` for `room` (what a joiner presents to the signaling service). Empty
/// for a malformed key (matches nothing).
pub fn door(key: &LinkKey, room: &str) -> String {
    mac(key, &[DOOR_LABEL, room])
        .map_or_else(String::new, |m| b64url(&m.finalize().into_bytes()[..16]))
}

/// What the host registers for a door: lowercase hex SHA-256 of the door string.
pub fn door_hash(door: &str) -> String {
    hex(&Sha256::digest(door.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Joiner → host: proves `key` over both fingerprints.
pub fn join_proof(key: &LinkKey, fp_joiner: &str, fp_host: &str) -> String {
    mac(key, &[JOIN_LABEL, fp_joiner, fp_host])
        .map_or_else(String::new, |m| b64url(&m.finalize().into_bytes()))
}

/// Host → joiner: proves the host holds `key` too.
pub fn host_proof(key: &LinkKey, fp_host: &str, fp_joiner: &str) -> String {
    mac(key, &[HOST_LABEL, fp_host, fp_joiner])
        .map_or_else(String::new, |m| b64url(&m.finalize().into_bytes()))
}

fn verify(m: Option<HmacSha256>, proof: &str) -> bool {
    let (Some(m), Some(p)) = (m, b64url_decode(proof)) else {
        return false;
    };
    // Constant time.
    m.verify_slice(&p).is_ok()
}

pub fn verify_join_proof(key: &LinkKey, fp_joiner: &str, fp_host: &str, proof: &str) -> bool {
    verify(mac(key, &[JOIN_LABEL, fp_joiner, fp_host]), proof)
}

pub fn verify_host_proof(key: &LinkKey, fp_host: &str, fp_joiner: &str, proof: &str) -> bool {
    verify(mac(key, &[HOST_LABEL, fp_host, fp_joiner]), proof)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// K = 00 01 .. 0f.
    const SECRET: &str = "AAECAwQFBgcICQoLDA0ODw";
    const ROOM: &str = "AbCdEfGhIjKlMnOpQrStUv";
    const FP_J: &str = "sha-256 AA:BB:CC";
    const FP_H: &str = "sha-256 11:22:33";

    fn key() -> LinkKey {
        LinkKey {
            version: KEY_V1,
            secret: SECRET.into(),
        }
    }

    /// Frozen vectors (docs/SHARING.md §4.2). Changing any of these breaks every link and
    /// member key in the wild.
    #[test]
    fn frozen_vectors() {
        let k = key();
        assert_eq!(key_bytes(&k).unwrap(), (0u8..16).collect::<Vec<_>>());
        let door = door(&k, ROOM);
        assert_eq!(door, "-NlMpoJxAQr27xLrrnWcyg");
        assert_eq!(
            door_hash(&door),
            "0ca35b191565d6d1aa58791582901a42b1ad1a79af44ae854d4b7c4b7dd63b28"
        );
        assert_eq!(
            join_proof(&k, FP_J, FP_H),
            "IIhhbob-mfbMMQj0IERpl9I5YvYUsGRkYgAGYT4B3xs"
        );
        assert_eq!(
            host_proof(&k, FP_H, FP_J),
            "Em9eKyuWCBC-Dygl1V2JRSJn5rlsPhSqcQ_mp-knftk"
        );
    }

    #[test]
    fn door_hash_is_sha256_hex_of_the_string() {
        // Same as the service's `sha256Hex` (services/signal/src/protocol.test.ts).
        assert_eq!(
            door_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn proofs_verify_and_bind_both_fingerprints() {
        let k = key();
        let p = join_proof(&k, FP_J, FP_H);
        assert!(verify_join_proof(&k, FP_J, FP_H, &p));
        assert!(!verify_join_proof(&k, FP_H, FP_J, &p));
        assert!(!verify_join_proof(&k, FP_J, "sha-256 FF", &p));
        let other = new_key().unwrap();
        assert!(!verify_join_proof(&other, FP_J, FP_H, &p));
        // The host proof is not the join proof (labels differ).
        let h = host_proof(&k, FP_H, FP_J);
        assert_ne!(h, p);
        assert!(verify_host_proof(&k, FP_H, FP_J, &h));
        assert!(!verify_join_proof(&k, FP_H, FP_J, &h));
        assert!(!verify_host_proof(&k, FP_H, FP_J, "not base64!"));
    }

    #[test]
    fn random_ids_and_keys_have_the_frozen_shapes() {
        let id = new_id().unwrap();
        assert!(valid_id(&id), "{id}");
        assert_ne!(id, new_id().unwrap());
        let k = new_key().unwrap();
        assert_eq!(parse_key(&k.to_string()), Some(k.clone()));
        assert_eq!(key_bytes(&k).map(|b| b.len()), Some(16));
        let t = new_host_token().unwrap();
        assert_eq!(t.len(), HOST_TOKEN_CHARS);
        assert!(
            t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(parse_key("2AAECAwQFBgcICQoLDA0ODw"), None);
        assert_eq!(parse_key("1short"), None);
        // Lenient trailing bits: any 22-char base64url secret is a key.
        let lenient = parse_key("10123456789_-abcdefghij").unwrap();
        assert_eq!(key_bytes(&lenient).map(|b| b.len()), Some(16));
    }
}
