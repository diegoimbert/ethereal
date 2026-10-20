//! The engine-side sample cache (`Library::sample_cache_*`). Files are content-addressed and
//! shared; manifests are kept per source URL (`<16 hex of sha256(url)>/`), so a test source
//! never replaces the real pack's manifest:
//!
//! ```text
//! files/<sha256>.<ext>      downloaded files (any source)
//! <source>/manifest.json    the signed envelope as downloaded (verified again on load)
//! sources.json              the source folders seen (their files survive another's update)
//! pins.json                 sha256s that projects reference: kept across updates
//! ```

use std::collections::BTreeSet;

use crate::store::{Library, StoreError};

use super::manifest::{self, Envelope, Manifest, PackFile};

const FILES: &str = "files";
const PINS: &str = "pins.json";
const SOURCES: &str = "sources.json";

pub struct Cache {
    dir: String,
}

fn read_set<L: Library>(library: &mut L, rel: &str) -> BTreeSet<String> {
    library
        .sample_cache_read(rel)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_set<L: Library>(library: &mut L, rel: &str, set: &BTreeSet<String>) -> bool {
    let bytes = serde_json::to_vec(set).unwrap_or_default();
    library.sample_cache_write(rel, &bytes).is_ok()
}

/// The sha256s of the files present (any source).
pub fn present<L: Library>(library: &mut L) -> BTreeSet<String> {
    library
        .sample_cache_list(FILES)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| name.split_once('.').map(|(h, _)| h.to_string()))
        .collect()
}

/// A cached file by its sha256 (whatever its extension), checked; a damaged one is deleted.
pub fn read_hash<L: Library>(library: &mut L, sha256: &str) -> Result<Vec<u8>, StoreError> {
    let not_found = || StoreError::NotFound(sha256.to_string());
    let name = library
        .sample_cache_list(FILES)
        .unwrap_or_default()
        .into_iter()
        .find(|n| n.split_once('.').is_some_and(|(h, _)| h == sha256))
        .ok_or_else(not_found)?;
    let rel = format!("{FILES}/{name}");
    let bytes = library.sample_cache_read(&rel)?;
    if manifest::sha256_hex(&bytes) != sha256 {
        let _ = library.sample_cache_remove(&rel);
        return Err(not_found());
    }
    Ok(bytes)
}

/// Keep `sha256` across pack updates (a project references it). Returns whether it was new.
#[allow(dead_code)] // projects pin the samples they reference (project references)
pub fn pin<L: Library>(library: &mut L, sha256: &str) -> bool {
    let mut pins = read_set(library, PINS);
    pins.insert(sha256.to_string()) && write_set(library, PINS, &pins)
}

impl Cache {
    pub fn for_url(url: &str) -> Self {
        Self {
            dir: manifest::sha256_hex(url.as_bytes())[..16].to_string(),
        }
    }

    fn manifest_rel(&self) -> String {
        format!("{}/manifest.json", self.dir)
    }

    /// The cached manifest, if it's there and still verifies with `keys`.
    pub fn load_manifest<L: Library>(
        &self,
        library: &mut L,
        keys: &[[u8; 32]],
    ) -> Option<Manifest> {
        let bytes = library.sample_cache_read(&self.manifest_rel()).ok()?;
        match manifest::verify(&bytes, keys) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!(target: "samples", "cached manifest dropped: {e}");
                None
            }
        }
    }

    pub fn store_manifest<L: Library>(
        &self,
        library: &mut L,
        envelope: &[u8],
    ) -> Result<(), StoreError> {
        library.sample_cache_write(&self.manifest_rel(), envelope)?;
        let mut sources = read_set(library, SOURCES);
        if sources.insert(self.dir.clone()) {
            write_set(library, SOURCES, &sources);
        }
        Ok(())
    }

    /// A cached file's bytes, checked against its sha256.
    pub fn read<L: Library>(&self, library: &mut L, f: &PackFile) -> Result<Vec<u8>, StoreError> {
        read_hash(library, &f.sha256)
    }

    pub fn write<L: Library>(
        &self,
        library: &mut L,
        f: &PackFile,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        // `f.path` is `files/<sha256>.<ext>` (validated).
        library.sample_cache_write(&f.path, bytes)
    }

    /// After an update to `m`: delete the files no cached manifest lists any more, unless
    /// pinned. Returns how many were deleted.
    pub fn collect_garbage<L: Library>(&self, library: &mut L, m: &Manifest) -> usize {
        let mut keep: BTreeSet<String> = m.files.iter().map(|f| f.sha256.clone()).collect();
        keep.extend(read_set(library, PINS));
        for other in read_set(library, SOURCES) {
            if other == self.dir {
                continue;
            }
            // Other sources' manifests only widen what's kept: no need to verify them.
            let listed = library
                .sample_cache_read(&format!("{other}/manifest.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Envelope>(&b).ok())
                .and_then(|e| serde_json::from_str::<Manifest>(&e.manifest).ok());
            if let Some(o) = listed {
                keep.extend(o.files.into_iter().map(|f| f.sha256));
            }
        }
        let mut removed = 0;
        for name in library.sample_cache_list(FILES).unwrap_or_default() {
            let hash = name.split_once('.').map_or(name.as_str(), |(h, _)| h);
            if keep.contains(hash) {
                continue;
            }
            if library
                .sample_cache_remove(&format!("{FILES}/{name}"))
                .is_ok()
            {
                removed += 1;
            }
        }
        removed
    }
}
