//! `MediaRef.hash`: a fast, deterministic, non-cryptographic 128-bit content hash (hex),
//! used to key peak caches and to dedupe imports. Stable across platforms and versions
//! (it names cache files); change it only together with a cache-format bump.

const P1: u64 = 0x9E37_79B9_7F4A_7C15;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;

#[inline]
fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^ (x >> 33)
}

/// Hash `bytes` to 32 lowercase hex chars.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut a: u64 = P1 ^ bytes.len() as u64;
    let mut b: u64 = P2.wrapping_add(bytes.len() as u64);
    let mut chunks = bytes.chunks_exact(16);
    for c in &mut chunks {
        let x = u64::from_le_bytes(c[..8].try_into().expect("8 bytes"));
        let y = u64::from_le_bytes(c[8..].try_into().expect("8 bytes"));
        a = (a ^ x.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1);
        b = (b ^ y.wrapping_mul(P1)).rotate_left(29).wrapping_mul(P3);
    }
    let mut tail = [0u8; 16];
    let rest = chunks.remainder();
    tail[..rest.len()].copy_from_slice(rest);
    let x = u64::from_le_bytes(tail[..8].try_into().expect("8 bytes"));
    let y = u64::from_le_bytes(tail[8..].try_into().expect("8 bytes"));
    a = mix(a ^ x ^ (rest.len() as u64));
    b = mix(b ^ y ^ a);
    a = mix(a ^ b);
    format!("{a:016x}{b:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_sensitive() {
        let h = content_hash(b"hello world, this is a test");
        assert_eq!(h.len(), 32);
        assert_eq!(h, content_hash(b"hello world, this is a test"));
        assert_ne!(h, content_hash(b"hello world, this is a tesu"));
        assert_ne!(content_hash(b""), content_hash(b"\0"));
        assert_ne!(content_hash(&[0u8; 16]), content_hash(&[0u8; 17]));
    }
}
