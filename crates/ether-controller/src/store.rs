//! Engine-side persistence: the [`ProjectStore`] and the sample [`Library`].
//!
//! **Only the engine/controller side touches files.** The UI may be on another machine and
//! addresses everything by id (`ProjectId`, `MediaId`) or by location-relative path.
//!
//! Layout under `projects_root`:
//!
//! ```text
//! <projects_root>/<project-uuid>/project.ether   the document (display name inside)
//!                               /media/          imported audio (copied in; self-contained)
//!                               /cache/          peaks, decoded audio (regenerable)
//! ```
//!
//! `projects_root` comes from engine/host config: default `~/Documents/Ethereal/Projects`;
//! in dev builds `<data_dir>/ethereal-dev/<instance>/projects` (per-instance isolation).
//!
//! Implementations (owned by the host nodes):
//! - native (`ether-native`): folders on disk under `projects_root`;
//! - web (`ether-wasm`): OPFS, accessed from the engine's Worker (still engine-side).
//!
//! Paths passed to these traits are always *relative* (to a project folder or a library
//! root). Implementations must reject absolute paths and `..` components.

use ether_core::protocol::media::{BrowseRoot, DirectoryListing};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::ProjectSummary;
use ether_core::protocol::share::ProjectShareInfo;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("io: {0}")]
    Io(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Project folders keyed by `ProjectId`.
pub trait ProjectStore {
    /// All stored projects (name read from each `project.ether`), newest first.
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError>;

    /// Create an empty project folder (with `media/`, `cache/`). Fails if it exists.
    fn create(&mut self, id: ProjectId) -> Result<(), StoreError>;

    /// Read `project.ether`.
    fn load(&mut self, id: ProjectId) -> Result<String, StoreError>;

    /// Atomically replace `project.ether` (write temp + rename). Returns the new summary.
    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError>;

    /// Copy the whole folder (document + media; cache optional) to `to`, **except
    /// [`SHARE_FILE`]** (a copy is a private project). The caller then rewrites the copy's
    /// name via `load`/`save`.
    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError>;

    /// Delete the project folder.
    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError>;

    /// Read a file relative to the project folder (e.g. `media/<file>`).
    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError>;

    /// Write a file relative to the project folder (media import, caches).
    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// List a folder relative to the project folder (browse location `ProjectMedia`).
    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError>;

    // --- Roadmap v2 (contracts-2), defaulted so existing stores keep compiling ------------

    /// `export`: store a finished export as `<project>/exports/<file_name>` (see
    /// [`export_path`]) and return that project-relative path. `Err(Unsupported)` (the
    /// default) means this host doesn't keep exports on disk: the controller offers the
    /// bytes as a download (`ExportResult::Download`) instead (web, remote).
    fn write_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, StoreError> {
        let _ = (id, file_name, bytes);
        Err(StoreError::Unsupported(
            "exports are delivered as downloads".into(),
        ))
    }

    /// `remote-engine`: start staging an upload (`Media::BeginUpload`) outside any project
    /// (native: `<projects_root>/.uploads/<upload>`), `size` bytes expected. Replaces a
    /// previous upload with the same id.
    fn begin_upload(&mut self, upload: &str, size: u64) -> Result<(), StoreError> {
        let _ = (upload, size);
        Err(StoreError::Unsupported("uploads".into()))
    }

    /// Append bytes at `offset` (must equal the bytes received so far). Returns the new total.
    fn append_upload(
        &mut self,
        upload: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, StoreError> {
        let _ = (upload, offset, bytes);
        Err(StoreError::Unsupported("uploads".into()))
    }

    /// The complete staged bytes (read by `Media::Import { source: Upload }`).
    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        let _ = upload;
        Err(StoreError::Unsupported("uploads".into()))
    }

    /// Drop a staged upload (cancel, after import, on disconnect). Missing = `Ok`.
    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        let _ = upload;
        Ok(())
    }

    // --- v0.3 (contracts-4), defaulted ---

    /// `project-versions`: delete a file relative to the project folder (old versions, the
    /// session marker). Missing = `Ok`. Default: unsupported (the node implements it in the
    /// native, OPFS and memory stores).
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        let _ = (id, rel_path);
        Err(StoreError::Unsupported("removing project files".into()))
    }
}

/// Project-relative path of an export file (`exports/<file_name>`): `file_name` must be a
/// single, non-hidden path segment.
pub fn export_path(file_name: &str) -> Result<String, StoreError> {
    if file_name.is_empty() || file_name.starts_with('.') || file_name.contains(['/', '\\', '\0']) {
        return Err(StoreError::InvalidPath(file_name.to_string()));
    }
    Ok(format!(
        "{}/{file_name}",
        ether_core::protocol::model::file::EXPORTS_DIR
    ))
}

/// Engine-visible sample library folders (configured on the engine side, never by path
/// from the UI).
pub trait Library {
    /// Library roots (`BrowseLocation::Library { id }`).
    fn roots(&self) -> Vec<BrowseRoot>;

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError>;

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError>;

    // --- v0.2 (contracts-3), defaulted ---

    /// Write a file in a writable root (the user library: presets, the browser index).
    /// Creates parent folders. Default: unsupported (read-only library).
    fn write_file(&mut self, root: &str, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let _ = (root, rel_path, bytes);
        Err(StoreError::Unsupported(
            "the library is read-only on this host".into(),
        ))
    }

    /// Delete a file (user presets). Default: unsupported.
    fn remove_file(&mut self, root: &str, rel_path: &str) -> Result<(), StoreError> {
        let _ = (root, rel_path);
        Err(StoreError::Unsupported(
            "the library is read-only on this host".into(),
        ))
    }

    /// Rename/move a file within a root (user presets). Default: unsupported.
    fn rename_file(&mut self, root: &str, from: &str, to: &str) -> Result<(), StoreError> {
        let _ = (root, from, to);
        Err(StoreError::Unsupported(
            "the library is read-only on this host".into(),
        ))
    }

    /// The writable user-library root id (presets, index), if any. Default: none.
    fn user_root(&self) -> Option<String> {
        None
    }

    /// `browser-v2`: add a user folder by absolute engine-side path; returns its root id.
    /// Native only. Default: unsupported.
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "user folders are not available on this host".into(),
        ))
    }

    /// `browser-v2`: forget a user folder. Default: unsupported.
    fn remove_folder(&mut self, root: &str) -> Result<(), StoreError> {
        let _ = root;
        Err(StoreError::Unsupported(
            "user folders are not available on this host".into(),
        ))
    }

    /// `media-references`: the absolute engine-side path of a library file, for an external
    /// reference (`MediaLocation::External`). `None` = cannot be referenced in place (web,
    /// remote): the import copies it into the project instead.
    fn external_path(&self, root: &str, rel_path: &str) -> Option<String> {
        let _ = (root, rel_path);
        None
    }

    /// `media-references`: the entries of an absolute engine-side folder (the Relink
    /// dialog's folder search): `(absolute path, is_dir)` per entry, hidden entries left out.
    /// Default: unsupported (hosts without OS files).
    fn list_external_dir(&mut self, path: &str) -> Result<Vec<(String, bool)>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "external folders are not available on this host".into(),
        ))
    }

    /// `media-references`: read an external reference by its absolute path. Default:
    /// unsupported.
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "external media is not available on this host".into(),
        ))
    }
}

/// base-115 (`recents-shared`): the host-local sharing file next to `project.ether`
/// (docs/SHARING.md §4.5). Every store reads it into [`ProjectSummary::share`], never copies
/// it on `duplicate` (so `SaveAs`/`Duplicate` make a private project) and deletes it with the
/// project folder.
pub use ether_collab::share::file::SHARE_FILE;

/// At most this many participants in [`ProjectShareInfo::participants`] (Recents avatars).
pub const SHARE_SUMMARY_PARTICIPANTS: usize = 8;

/// The Recents view of a project's `share.json` bytes: role, host name, last known
/// participants, `active`. The secrets (room, host token, link and member keys) never leave
/// this function. `None` when the file is unreadable (a newer or corrupt file lists the
/// project as not shared rather than hiding it).
///
/// - **Host**: `host_name` is empty (the host is this user, whose identity lives in the UI);
///   `participants` are the remembered members, most recently seen first. `active` = some
///   link or member key is still valid (`Stop` deletes the file or leaves it with neither).
/// - **Copy**: `participants` as last synced (host first). `active` = the member key is still
///   stored: when the host stops sharing or removes this member, the copy's `key` is cleared
///   (empty), which Recents shows as "Sharing ended".
pub fn share_info(share_json: &[u8]) -> Option<ProjectShareInfo> {
    use ether_collab::share::file::ShareFile;
    use ether_core::protocol::share::{ParticipantRole, ParticipantSummary, ShareRole};

    let file: ShareFile = serde_json::from_slice(share_json).ok()?;
    Some(match file {
        ShareFile::Host(h) => {
            let mut members: Vec<_> = h.members.iter().collect();
            members.sort_by(|a, b| b.last_seen_ms.total_cmp(&a.last_seen_ms));
            ProjectShareInfo {
                role: ParticipantRole::Host,
                host_name: String::new(),
                participants: members
                    .into_iter()
                    .take(SHARE_SUMMARY_PARTICIPANTS)
                    .map(|m| ParticipantSummary {
                        name: m.name.clone(),
                        color: m.color,
                    })
                    .collect(),
                active: h.edit_key.is_some() || h.listen_key.is_some() || !h.members.is_empty(),
                last_synced_ms: None,
            }
        }
        ShareFile::Copy(c) => ProjectShareInfo {
            role: match c.role {
                ShareRole::Edit => ParticipantRole::Edit,
                ShareRole::Listen => ParticipantRole::Listen,
            },
            host_name: c.host_name,
            participants: c
                .participants
                .into_iter()
                .take(SHARE_SUMMARY_PARTICIPANTS)
                .collect(),
            active: !c.key.is_empty(),
            last_synced_ms: c.last_synced_ms,
        },
    })
}

/// Validate a relative path from the UI or a document: no absolute paths, drive letters,
/// backslashes, `.`/`..` or empty components. `""` (a location root) is accepted.
pub fn check_relative_path(path: &str) -> Result<(), StoreError> {
    if path.is_empty() {
        return Ok(());
    }
    let bad = path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        return Err(StoreError::InvalidPath(path.to_string()));
    }
    Ok(())
}

/// Classify a file by extension (for directory listings).
pub fn file_kind(name: &str) -> ether_core::protocol::media::FileKind {
    use ether_core::protocol::media::FileKind;
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "wav" | "wave" | "aif" | "aiff" | "aifc" | "flac" | "mp3" | "ogg" | "oga" => {
            FileKind::Audio
        }
        "mid" | "midi" => FileKind::Midi,
        _ => FileKind::Other,
    }
}

/// Fixture `share.json`s (the frozen shape of `ether_collab::share::file`) for store tests in
/// this crate and the host crates.
#[doc(hidden)]
pub mod share_fixtures {
    /// A host's file: edit link on, listen link off, three members (last seen 1, 3, 2).
    pub const HOST: &str = r#"{
  "kind": "Host", "version": 1, "room": "AAAAAAAAAAAAAAAAAAAAAA", "signal_url": null,
  "host_token": "SECRET-HOST-TOKEN", "edit_key": "1SECRET-EDIT-KEY", "listen_key": null,
  "members": [
    {"member": "m1", "key": "1SECRET-M1", "role": "Edit", "name": "Ada", "color": 1, "last_seen_ms": 1.0},
    {"member": "m2", "key": "1SECRET-M2", "role": "Listen", "name": "Tom", "color": 2, "last_seen_ms": 3.0},
    {"member": "m3", "key": "1SECRET-M3", "role": "Edit", "name": "Kim", "color": 3, "last_seen_ms": 2.0}
  ],
  "sites": {"7": 12}, "resume": true
}"#;

    /// An offline copy of Diego's project, joined with an edit link.
    pub const COPY: &str = r#"{
  "kind": "Copy", "version": 1, "room": "AAAAAAAAAAAAAAAAAAAAAA", "signal_url": null,
  "member": "m1", "key": "1SECRET-MEMBER-KEY", "role": "Edit", "host_name": "Diego",
  "participants": [{"name": "Diego", "color": 4}, {"name": "Ada", "color": 1}],
  "last_synced_ms": 1700000000000.0
}"#;

    /// A listen copy after the host stopped sharing (the member key is cleared).
    pub const COPY_ENDED: &str = r#"{
  "kind": "Copy", "version": 1, "room": "AAAAAAAAAAAAAAAAAAAAAA", "signal_url": null,
  "member": "m1", "key": "", "role": "Listen", "host_name": "Diego",
  "participants": [{"name": "Diego", "color": 4}],
  "last_synced_ms": null
}"#;

    /// Every secret in the fixtures (none may reach a `ProjectSummary`).
    pub const SECRETS: &[&str] = &["SECRET", "AAAAAAAAAAAAAAAAAAAAAA"];
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::share::ParticipantRole;

    #[test]
    fn share_info_of_a_host_file() {
        let info = share_info(share_fixtures::HOST.as_bytes()).unwrap();
        assert_eq!(info.role, ParticipantRole::Host);
        assert_eq!(info.host_name, "");
        let names: Vec<_> = info.participants.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Tom", "Kim", "Ada"], "most recently seen first");
        assert!(info.active);
        assert_eq!(info.last_synced_ms, None);
    }

    #[test]
    fn share_info_of_copies() {
        let info = share_info(share_fixtures::COPY.as_bytes()).unwrap();
        assert_eq!(info.role, ParticipantRole::Edit);
        assert_eq!(info.host_name, "Diego");
        assert_eq!(info.participants.len(), 2);
        assert_eq!(info.participants[0].name, "Diego");
        assert!(info.active);
        assert_eq!(info.last_synced_ms, Some(1_700_000_000_000.0));

        let ended = share_info(share_fixtures::COPY_ENDED.as_bytes()).unwrap();
        assert_eq!(ended.role, ParticipantRole::Listen);
        assert!(!ended.active);
    }

    #[test]
    fn share_info_never_carries_secrets() {
        for f in [
            share_fixtures::HOST,
            share_fixtures::COPY,
            share_fixtures::COPY_ENDED,
        ] {
            let json = serde_json::to_string(&share_info(f.as_bytes()).unwrap()).unwrap();
            for s in share_fixtures::SECRETS {
                assert!(!json.contains(s), "{s} leaked: {json}");
            }
        }
    }

    #[test]
    fn share_info_caps_participants_and_reads_stopped_hosts() {
        let members: Vec<String> = (0..12)
            .map(|i| {
                format!(
                    r#"{{"member":"m{i}","key":"k","role":"Edit","name":"P{i}","color":{i},"last_seen_ms":{i}.0}}"#
                )
            })
            .collect();
        let host = format!(
            r#"{{"kind":"Host","version":1,"room":"r","signal_url":null,"host_token":"t",
                "edit_key":null,"listen_key":null,"members":[{}],"resume":false}}"#,
            members.join(",")
        );
        let info = share_info(host.as_bytes()).unwrap();
        assert_eq!(info.participants.len(), SHARE_SUMMARY_PARTICIPANTS);
        assert_eq!(info.participants[0].name, "P11");
        assert!(info.active, "members still hold keys");

        let stopped = r#"{"kind":"Host","version":1,"room":"r","signal_url":null,"host_token":"t",
            "edit_key":null,"listen_key":null,"members":[],"resume":false}"#;
        assert!(!share_info(stopped.as_bytes()).unwrap().active);
    }

    #[test]
    fn unreadable_share_files_are_ignored() {
        assert_eq!(share_info(b""), None);
        assert_eq!(share_info(b"{\"kind\":\"Future\"}"), None);
        assert_eq!(share_info(b"not json"), None);
    }
}
