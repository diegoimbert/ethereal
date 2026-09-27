//! Stable `ParamId` (u32) ↔ `AUParameterAddress` (u64) mapping, and the versioned state blob
//! that carries it. Platform-independent (unit-tested everywhere).
//!
//! # Param ids
//! Every AU is hosted through `AUAudioUnit` (v3 natively, v2 through Apple's v2 bridge), so
//! parameters are always `AUParameterTree` addresses:
//! - v3 units: whatever the unit declares;
//! - v2 units via the bridge: `kAudioUnitScope_Global` element 0 params have address =
//!   `AudioUnitParameterID` (fits u32); params of other scopes/elements get addresses packed
//!   by the bridge above 32 bits.
//!
//! Mapping (built in tree order, deterministic):
//! 1. Entries of the table saved in the state blob (address still present, id unused) are
//!    reused verbatim, so a reloaded project gets exactly the ids it was saved with.
//! 2. Addresses `<= u32::MAX` map to themselves (id = address).
//! 3. Larger addresses get `fnv1a32(address as little-endian bytes)`, probing `+1` (wrapping)
//!    past ids already taken or reserved by rule 2 (all small addresses are reserved before
//!    any hash is assigned).
//!
//! The full address→id table is written into every saved state ([`encode_state`]).

/// Address → id table (sorted by address; ids unique).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamMap {
    by_addr: Vec<(u64, u32)>,
    by_id: Vec<(u32, u64)>,
}

/// FNV-1a (32-bit) of the address' little-endian bytes.
pub fn fnv1a32(address: u64) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in address.to_le_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl ParamMap {
    /// Map `addresses` (tree order), reusing `saved` (address, id) entries where possible.
    pub fn build(addresses: &[u64], saved: &[(u64, u32)]) -> Self {
        use std::collections::{HashMap, HashSet};
        let mut ids: HashMap<u64, u32> = HashMap::with_capacity(addresses.len());
        let mut used: HashSet<u32> = HashSet::with_capacity(addresses.len());
        let present: HashSet<u64> = addresses.iter().copied().collect();
        for &(addr, id) in saved {
            if present.contains(&addr) && !ids.contains_key(&addr) && used.insert(id) {
                ids.insert(addr, id);
            }
        }
        for &addr in addresses {
            if ids.contains_key(&addr) {
                continue;
            }
            if let Ok(small) = u32::try_from(addr)
                && used.insert(small)
            {
                ids.insert(addr, small);
            }
        }
        // Reserve every small address (even one whose id was taken by a saved entry) so a
        // hashed id never shadows a direct one.
        let reserved: HashSet<u32> = addresses
            .iter()
            .filter_map(|a| u32::try_from(*a).ok())
            .collect();
        for &addr in addresses {
            if ids.contains_key(&addr) {
                continue;
            }
            let mut id = fnv1a32(addr);
            while used.contains(&id) || (reserved.contains(&id) && u64::from(id) != addr) {
                id = id.wrapping_add(1);
            }
            used.insert(id);
            ids.insert(addr, id);
        }
        let mut by_addr: Vec<(u64, u32)> = ids.into_iter().collect();
        by_addr.sort_unstable();
        let mut by_id: Vec<(u32, u64)> = by_addr.iter().map(|&(a, i)| (i, a)).collect();
        by_id.sort_unstable();
        Self { by_addr, by_id }
    }

    pub fn id(&self, address: u64) -> Option<u32> {
        self.by_addr
            .binary_search_by_key(&address, |e| e.0)
            .ok()
            .map(|i| self.by_addr[i].1)
    }

    pub fn address(&self, id: u32) -> Option<u64> {
        self.by_id
            .binary_search_by_key(&id, |e| e.0)
            .ok()
            .map(|i| self.by_id[i].1)
    }

    /// `(address, id)` sorted by address.
    pub fn entries(&self) -> &[(u64, u32)] {
        &self.by_addr
    }

    pub fn len(&self) -> usize {
        self.by_addr.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_addr.is_empty()
    }
}

/// `(address, id)` entries of a saved mapping.
pub type ParamTable = Vec<(u64, u32)>;

const MAGIC: &[u8; 4] = b"EAUS";
const VERSION: u8 = 1;

/// State blob: `"EAUS"`, version `1`, `u32` entry count, entries (`u64` address, `u32` id),
/// `u32` plist length, the plist bytes (`fullStateForDocument` as a binary plist). Integers
/// little-endian.
pub fn encode_state(map: &[(u64, u32)], plist: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(13 + map.len() * 12 + plist.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    for &(addr, id) in map {
        out.extend_from_slice(&addr.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
    }
    out.extend_from_slice(&(plist.len() as u32).to_le_bytes());
    out.extend_from_slice(plist);
    out
}

/// Inverse of [`encode_state`]: `(table, plist)`.
pub fn decode_state(blob: &[u8]) -> Result<(ParamTable, &[u8]), String> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
        if b.len() < n {
            return Err("truncated AU state".into());
        }
        let (head, rest) = b.split_at(n);
        *b = rest;
        Ok(head)
    }
    fn u32_le(b: &mut &[u8]) -> Result<u32, String> {
        Ok(u32::from_le_bytes(take(b, 4)?.try_into().expect("4 bytes")))
    }
    let mut b = blob;
    if take(&mut b, 4)? != MAGIC {
        return Err("not an Ethereal AU state blob".into());
    }
    let version = take(&mut b, 1)?[0];
    if version != VERSION {
        return Err(format!("unsupported AU state version {version}"));
    }
    let n = u32_le(&mut b)? as usize;
    if n > b.len() / 12 {
        return Err("truncated AU state".into());
    }
    let mut table = Vec::with_capacity(n);
    for _ in 0..n {
        let addr = u64::from_le_bytes(take(&mut b, 8)?.try_into().expect("8 bytes"));
        table.push((addr, u32_le(&mut b)?));
    }
    let len = u32_le(&mut b)? as usize;
    let plist = take(&mut b, len)?;
    if !b.is_empty() {
        return Err("trailing bytes in AU state".into());
    }
    Ok((table, plist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_addresses_map_directly() {
        let m = ParamMap::build(&[0, 1, 7, 42], &[]);
        for a in [0u64, 1, 7, 42] {
            assert_eq!(m.id(a), Some(a as u32));
            assert_eq!(m.address(a as u32), Some(a));
        }
        assert_eq!(m.len(), 4);
        assert_eq!(m.id(3), None);
    }

    #[test]
    fn large_addresses_hash_stably_and_avoid_collisions() {
        let big = [1u64 << 40, (2u64 << 56) | 5, u64::MAX];
        let mut addrs = vec![3u64];
        addrs.extend(big);
        let m = ParamMap::build(&addrs, &[]);
        let m2 = ParamMap::build(&addrs, &[]);
        assert_eq!(m, m2, "deterministic");
        for a in big {
            assert_eq!(m.id(a), Some(fnv1a32(a)));
        }
        // Known vector: FNV-1a 32 of eight zero bytes.
        assert_eq!(fnv1a32(0), 0x9BE1_7165);

        // Force a collision: a small address equal to a big one's hash is kept direct and
        // the big one probes past it.
        let h = fnv1a32(1u64 << 40);
        let m = ParamMap::build(&[1u64 << 40, u64::from(h)], &[]);
        assert_eq!(m.id(u64::from(h)), Some(h));
        assert_eq!(m.id(1u64 << 40), Some(h.wrapping_add(1)));
    }

    #[test]
    fn saved_table_wins() {
        let addrs = [1u64 << 40, 5];
        let saved = [(1u64 << 40, 99u32), (1234u64, 7u32)];
        let m = ParamMap::build(&addrs, &saved);
        assert_eq!(m.id(1u64 << 40), Some(99));
        assert_eq!(m.id(5), Some(5));
        assert_eq!(m.address(7), None, "absent address dropped");
        // Duplicate ids in a corrupt table don't produce duplicate ids.
        let m = ParamMap::build(&[10, 11], &[(10, 1), (11, 1)]);
        assert_eq!(m.id(10), Some(1));
        assert_eq!(m.id(11), Some(11));
    }

    #[test]
    fn state_round_trip() {
        let table = vec![(1u64, 1u32), (1u64 << 40, 77)];
        let blob = encode_state(&table, b"bplist00xyz");
        let (t, p) = decode_state(&blob).unwrap();
        assert_eq!(t, table);
        assert_eq!(p, b"bplist00xyz");
        assert!(decode_state(b"nope").is_err());
        assert!(decode_state(&blob[..blob.len() - 1]).is_err());
        let mut extra = blob.clone();
        extra.push(0);
        assert!(decode_state(&extra).is_err());
        let mut v2 = blob;
        v2[4] = 2;
        assert!(decode_state(&v2).unwrap_err().contains("version"));
    }
}
