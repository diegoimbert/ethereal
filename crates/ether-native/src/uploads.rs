//! Upload staging for the native store (roadmap v2, owned by the `remote-engine` node; see
//! `docs/ROADMAP.md`). `DiskStore`'s `ProjectStore::{begin,append,read,discard}_upload`
//! delegate here.
//!
//! Layout: `<projects_root>/.uploads/<upload>` (a partial file + its expected size). Upload
//! ids are client-chosen: validate them (single safe segment) before touching the disk.
//! Stale uploads (no chunk for a while, or on server start) are deleted.

use std::path::Path;

use ether_controller::store::StoreError;

fn unsupported() -> StoreError {
    StoreError::Unsupported("uploads are not implemented yet (remote-engine node)".into())
}

pub fn begin(projects_root: &Path, upload: &str, size: u64) -> Result<(), StoreError> {
    let _ = (projects_root, upload, size);
    Err(unsupported())
}

pub fn append(
    projects_root: &Path,
    upload: &str,
    offset: u64,
    bytes: &[u8],
) -> Result<u64, StoreError> {
    let _ = (projects_root, upload, offset, bytes);
    Err(unsupported())
}

pub fn read(projects_root: &Path, upload: &str) -> Result<Vec<u8>, StoreError> {
    let _ = (projects_root, upload);
    Err(unsupported())
}

pub fn discard(projects_root: &Path, upload: &str) -> Result<(), StoreError> {
    let _ = (projects_root, upload);
    Ok(())
}
