//! Writes the demo samples ([`ether_media::demo::demo_samples`]) into a library folder.

use std::io;
use std::path::Path;

/// Write the demo samples into `dir` (creating it). Existing files are left alone.
pub fn ensure(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, bytes) in ether_media::demo::demo_samples() {
        let path = dir.join(name);
        if !path.exists() {
            crate::store::atomic_write(&path, &bytes)
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_the_demo_samples_once() {
        let tmp = crate::test_util::TempDir::new("demo-samples");
        super::ensure(tmp.path()).unwrap();
        let n = std::fs::read_dir(tmp.path()).unwrap().count();
        assert_eq!(n, 5);
        // Idempotent.
        super::ensure(tmp.path()).unwrap();
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 5);
    }
}
