//! Portable project bundles (`Project::ExportBundle` / `ImportBundle`, base-114).
//!
//! A bundle is one `.ether` file: a plain ZIP archive whose entries are **stored** (not
//! compressed: audio barely compresses, and storing keeps the code small and wasm-friendly)
//! with UTF-8 names:
//!
//! ```text
//! project.ether        the document
//! media/<file>         the project's own media (subfolders kept)
//! ```
//!
//! Any ZIP tool opens it. `unpack` reads stored entries only (a bundle re-zipped with
//! compression is refused with a clear message) and ignores entries outside that layout
//! (e.g. `__MACOSX/`). ZIP64 is not supported: a bundle is at most 4 GiB.

use crate::store::check_relative_path;

/// The document entry.
pub const DOCUMENT: &str = "project.ether";
/// Prefix of media entries.
pub const MEDIA_PREFIX: &str = "media/";

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
/// General purpose flag bit 11: names are UTF-8.
const UTF8_FLAG: u16 = 1 << 11;
/// ZIP 2.0 (what plain stored entries need).
const VERSION: u16 = 20;
/// 1980-01-01 00:00 in MS-DOS format (the time is not meaningful in a bundle).
const DOS_DATE: u16 = (1 << 5) | 1;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// CRC-32 (IEEE), as ZIP uses it.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c = CRC_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn too_large() -> String {
    "the project is too large for a bundle (4 GiB at most)".into()
}

/// Pack `(name, bytes)` entries into a stored ZIP archive.
pub fn pack(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if entries.len() > u16::MAX as usize {
        return Err("too many files for a bundle".into());
    }
    let total: usize = entries
        .iter()
        .map(|(n, b)| 76 + 2 * n.len() + b.len())
        .sum();
    if total + 22 > u32::MAX as usize {
        return Err(too_large());
    }
    let mut out = Vec::with_capacity(total + 22);
    let mut central = Vec::new();
    for (name, bytes) in entries {
        let offset = out.len() as u32;
        let crc = crc32(bytes);
        let (size, name_len) = (bytes.len() as u32, name.len() as u16);
        put32(&mut out, LOCAL_SIG);
        put16(&mut out, VERSION);
        put16(&mut out, UTF8_FLAG);
        put16(&mut out, 0); // stored
        put16(&mut out, 0); // time
        put16(&mut out, DOS_DATE);
        put32(&mut out, crc);
        put32(&mut out, size);
        put32(&mut out, size);
        put16(&mut out, name_len);
        put16(&mut out, 0); // extra
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(bytes);

        put32(&mut central, CENTRAL_SIG);
        put16(&mut central, VERSION); // made by
        put16(&mut central, VERSION); // needed
        put16(&mut central, UTF8_FLAG);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, DOS_DATE);
        put32(&mut central, crc);
        put32(&mut central, size);
        put32(&mut central, size);
        put16(&mut central, name_len);
        put16(&mut central, 0); // extra
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk
        put16(&mut central, 0); // internal attributes
        put32(&mut central, 0); // external attributes
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_offset, cd_size) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    put32(&mut out, END_SIG);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, cd_size);
    put32(&mut out, cd_offset);
    put16(&mut out, 0); // comment
    Ok(out)
}

/// Is `bytes` a ZIP archive (rather than a bare `project.ether` document)?
pub fn is_archive(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == LOCAL_SIG
}

fn rd16(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(corrupt)
}
fn rd32(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(corrupt)
}
fn corrupt() -> String {
    "this file is not a valid Ethereal project bundle".into()
}

/// The bundle's document and media entries, `(name, bytes)` in archive order. Names are
/// checked (relative, no `..`); entries outside the bundle layout and folders are skipped.
pub fn unpack(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    // End of central directory: the last signature within the maximal comment length.
    let min = bytes.len().saturating_sub(22 + u16::MAX as usize);
    let end = (min..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| rd32(bytes, i) == Ok(END_SIG))
        .ok_or_else(corrupt)?;
    let count = rd16(bytes, end + 10)? as usize;
    let mut at = rd32(bytes, end + 16)? as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        if rd32(bytes, at)? != CENTRAL_SIG {
            return Err(corrupt());
        }
        let method = rd16(bytes, at + 10)?;
        let crc = rd32(bytes, at + 16)?;
        let size = rd32(bytes, at + 20)? as usize;
        let name_len = rd16(bytes, at + 28)? as usize;
        let extra_len = rd16(bytes, at + 30)? as usize;
        let comment_len = rd16(bytes, at + 32)? as usize;
        let local = rd32(bytes, at + 42)? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(corrupt)?;
        let name = std::str::from_utf8(name)
            .map_err(|_| corrupt())?
            .to_string();
        at += 46 + name_len + extra_len + comment_len;

        let wanted = name == DOCUMENT || (name.starts_with(MEDIA_PREFIX) && !name.ends_with('/'));
        if !wanted {
            continue;
        }
        check_relative_path(&name).map_err(|_| format!("bad file name in bundle: {name}"))?;
        if method != 0 {
            return Err(format!(
                "{name} is compressed: bundles store files uncompressed (export it again from Ethereal)"
            ));
        }
        if rd32(bytes, local)? != LOCAL_SIG {
            return Err(corrupt());
        }
        let start =
            local + 30 + rd16(bytes, local + 26)? as usize + rd16(bytes, local + 28)? as usize;
        let data = bytes.get(start..start + size).ok_or_else(corrupt)?;
        if crc32(data) != crc {
            return Err(format!(
                "{name} is damaged in the bundle (checksum mismatch)"
            ));
        }
        out.push((name, data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn pack_unpack_round_trip() {
        let entries = vec![
            (DOCUMENT.to_string(), b"{}".to_vec()),
            ("media/kick.wav".to_string(), vec![1, 2, 3, 0, 255]),
            ("media/sub/ü.wav".to_string(), Vec::new()),
        ];
        let zip = pack(&entries).unwrap();
        assert!(is_archive(&zip));
        let back = unpack(&zip).unwrap();
        assert_eq!(back.len(), 3);
        for ((n, b), (n2, b2)) in entries.iter().zip(&back) {
            assert_eq!(n, n2);
            assert_eq!(b.as_slice(), *b2);
        }
    }

    #[test]
    fn rejects_damage_and_escapes() {
        let mut zip = pack(&[("media/a.wav".to_string(), vec![7; 16])]).unwrap();
        zip[45] ^= 0xff; // inside the data (30-byte header + 11-byte name)
        assert!(unpack(&zip).unwrap_err().contains("checksum"));
        let zip = pack(&[("media/../x".to_string(), vec![1])]).unwrap();
        assert!(unpack(&zip).is_err());
        assert!(unpack(b"not a zip").is_err());
        // Entries outside the layout are ignored.
        let zip = pack(&[("__MACOSX/x".to_string(), vec![1])]).unwrap();
        assert!(unpack(&zip).unwrap().is_empty());
    }
}
