//! Upload staging for the native store (roadmap v2, owned by the `remote-engine` node; see
//! `docs/ROADMAP.md`). `DiskStore`'s `ProjectStore::{begin,append,read,discard}_upload`
//! delegate here.
//!
//! Layout: `<projects_root>/.uploads/<upload>.part` (the bytes received so far) and
//! `<upload>.size` (the expected size, decimal). The folder starts with a dot, so the
//! project list and browsers never show it. Upload ids are client-chosen: they must be a
//! single safe segment (`[A-Za-z0-9_-]`, 1..=64 chars) before anything touches the disk.
//! Abandoned uploads are removed by [`remove_stale`] (the server calls it at start and
//! periodically); the controller also discards idle uploads it tracks.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ether_controller::store::StoreError;

/// Staging folder name under `projects_root`.
pub const UPLOADS_DIR: &str = ".uploads";

fn io_err(e: std::io::Error) -> StoreError {
    StoreError::Io(e.to_string())
}

/// `true` for ids that are safe as a single file-name segment.
pub fn valid_upload_id(upload: &str) -> bool {
    !upload.is_empty()
        && upload.len() <= 64
        && upload
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn paths(projects_root: &Path, upload: &str) -> Result<(PathBuf, PathBuf), StoreError> {
    if !valid_upload_id(upload) {
        return Err(StoreError::InvalidPath(format!("upload id {upload:?}")));
    }
    let dir = projects_root.join(UPLOADS_DIR);
    Ok((
        dir.join(format!("{upload}.part")),
        dir.join(format!("{upload}.size")),
    ))
}

fn expected_size(size_file: &Path, upload: &str) -> Result<u64, StoreError> {
    let s = fs::read_to_string(size_file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(format!("upload {upload}")),
        _ => io_err(e),
    })?;
    s.trim()
        .parse()
        .map_err(|_| StoreError::Io(format!("corrupt upload metadata for {upload}")))
}

fn received(part: &Path, upload: &str) -> Result<u64, StoreError> {
    fs::metadata(part)
        .map(|m| m.len())
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(format!("upload {upload}")),
            _ => io_err(e),
        })
}

/// Start (or restart) staging `upload`, `size` bytes expected.
pub fn begin(projects_root: &Path, upload: &str, size: u64) -> Result<(), StoreError> {
    let (part, size_file) = paths(projects_root, upload)?;
    fs::create_dir_all(projects_root.join(UPLOADS_DIR)).map_err(io_err)?;
    fs::write(&size_file, size.to_string()).map_err(io_err)?;
    fs::File::create(&part).map_err(io_err)?;
    Ok(())
}

/// Append `bytes` at `offset` (must equal the bytes received so far, and stay within the
/// expected size). Returns the new total.
pub fn append(
    projects_root: &Path,
    upload: &str,
    offset: u64,
    bytes: &[u8],
) -> Result<u64, StoreError> {
    let (part, size_file) = paths(projects_root, upload)?;
    let size = expected_size(&size_file, upload)?;
    let have = received(&part, upload)?;
    if offset != have {
        return Err(StoreError::Io(format!(
            "upload {upload}: chunk at {offset}, expected {have}"
        )));
    }
    let total = have + bytes.len() as u64;
    if total > size {
        return Err(StoreError::Io(format!(
            "upload {upload}: {total} bytes exceed the announced {size}"
        )));
    }
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&part)
        .map_err(io_err)?;
    f.write_all(bytes).map_err(io_err)?;
    Ok(total)
}

/// The complete staged bytes. `NotFound` for unknown ids, `Io` while incomplete.
pub fn read(projects_root: &Path, upload: &str) -> Result<Vec<u8>, StoreError> {
    let (part, size_file) = paths(projects_root, upload)?;
    let size = expected_size(&size_file, upload)?;
    let have = received(&part, upload)?;
    if have != size {
        return Err(StoreError::Io(format!(
            "upload {upload} is incomplete ({have} of {size} bytes)"
        )));
    }
    fs::read(&part).map_err(io_err)
}

/// Drop a staged upload. Missing = `Ok`.
pub fn discard(projects_root: &Path, upload: &str) -> Result<(), StoreError> {
    let (part, size_file) = paths(projects_root, upload)?;
    for f in [part, size_file] {
        match fs::remove_file(&f) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(e)),
        }
    }
    Ok(())
}

/// Delete staged files not modified for `max_age` (`Duration::ZERO`: everything, e.g. at
/// server start). Returns how many files were removed.
pub fn remove_stale(projects_root: &Path, max_age: Duration) -> usize {
    let Ok(rd) = fs::read_dir(projects_root.join(UPLOADS_DIR)) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in rd.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| now.duration_since(t).unwrap_or_default() >= max_age)
            .unwrap_or(true);
        if stale && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn stage_append_read_discard() {
        let tmp = TempDir::new("uploads");
        let root = tmp.path();
        begin(root, "u1", 5).unwrap();
        assert_eq!(append(root, "u1", 0, b"abc").unwrap(), 3);
        // Wrong offset (resend / gap) and overflow are rejected.
        assert!(append(root, "u1", 0, b"abc").is_err());
        assert!(append(root, "u1", 3, b"xyz").is_err());
        assert!(matches!(read(root, "u1"), Err(StoreError::Io(_))));
        assert_eq!(append(root, "u1", 3, b"de").unwrap(), 5);
        assert_eq!(read(root, "u1").unwrap(), b"abcde");
        discard(root, "u1").unwrap();
        discard(root, "u1").unwrap();
        assert!(matches!(read(root, "u1"), Err(StoreError::NotFound(_))));
        assert!(matches!(
            append(root, "u1", 0, b"a"),
            Err(StoreError::NotFound(_))
        ));
        // Restarting an upload truncates it.
        begin(root, "u2", 2).unwrap();
        append(root, "u2", 0, b"a").unwrap();
        begin(root, "u2", 2).unwrap();
        assert_eq!(append(root, "u2", 0, b"zz").unwrap(), 2);
        assert_eq!(read(root, "u2").unwrap(), b"zz");
    }

    #[test]
    fn rejects_unsafe_ids() {
        let tmp = TempDir::new("uploads-ids");
        for bad in ["", "../x", "a/b", ".hidden", "a b", &"x".repeat(65)] {
            assert!(
                matches!(begin(tmp.path(), bad, 1), Err(StoreError::InvalidPath(_))),
                "{bad:?}"
            );
        }
        assert!(valid_upload_id("01J8ZK3V9Q4N2B7XK6M1T0R5AS"));
    }

    #[test]
    fn removes_stale_uploads() {
        let tmp = TempDir::new("uploads-stale");
        begin(tmp.path(), "a", 1).unwrap();
        begin(tmp.path(), "b", 1).unwrap();
        assert_eq!(remove_stale(tmp.path(), Duration::from_secs(3600)), 0);
        assert_eq!(remove_stale(tmp.path(), Duration::ZERO), 4);
        assert!(matches!(
            read(tmp.path(), "a"),
            Err(StoreError::NotFound(_))
        ));
    }
}
