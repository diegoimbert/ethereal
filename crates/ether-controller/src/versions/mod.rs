//! Project versions and crash recovery (v0.3, owned by the `project-versions` node;
//! protocol `ether_protocol::versions`, CONTRACTS.md §13.11).
//!
//! - [`EtherController::version_command`]: every `Version` command (from `handlers.rs`).
//!   Versions are `.ether` documents at `versions/<id>.ether` in the project folder, written
//!   and listed through `ProjectStore::{write, read, list_dir}` and deleted with
//!   `ProjectStore::remove`, so native disk, web OPFS and the memory store all work. Version
//!   names live in `versions/names.json` (id → name). Only the document is versioned: other
//!   files in the project folder (media, `share.json`, exports) are never copied or restored.
//! - [`EtherController::versions_tick`]: rolling autosave versions (a version when the
//!   document changed since the last one, at most every `VERSION_INTERVAL_MS`, pruning to
//!   `MAX_AUTOSAVE_VERSIONS`).
//! - The session marker (`versions/.session`, holding this controller's session id) is
//!   written when a project is loaded (`versions_on_load`, from `project.rs`), removed when
//!   another project is loaded and when the controller is dropped (a clean close; skipped
//!   while panicking).
//! - `ListRecoverable` scans the stored projects for markers left by another session whose
//!   newest version is newer than `project.ether`. A project opened before the dialog asked
//!   (an auto-reopen) still counts: its stale marker is remembered when it is opened.

mod diff;

pub use diff::diff;

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::model::{ProjectId, file};
use ether_core::protocol::versions::{
    MAX_AUTOSAVE_VERSIONS, RecoveryInfo, VERSION_INTERVAL_MS, VersionCommand, VersionEvent,
    VersionId, VersionInfo, VersionKind,
};
use ether_core::protocol::{ErrorCode, Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, no_project, notify, store_err};
use crate::store::{Library, ProjectStore, StoreError};
use crate::tx::{CmdResult, cmd_err, invalid, invalid_state};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Folder of the versions, relative to the project folder.
pub const VERSIONS_DIR: &str = "versions";
/// The crash-recovery session marker (project-relative).
pub const SESSION_MARKER: &str = "versions/.session";
/// Version names (project-relative): a JSON object `{ "<id>": "<name>" }`.
pub const NAMES_FILE: &str = "versions/names.json";
const EXT: &str = ".ether";
/// Longest version name kept (characters).
const MAX_NAME_CHARS: usize = 120;

/// Controller-side state (not in the document).
#[derive(Debug, Default)]
pub(crate) struct VersionsState {
    /// The project the session marker was written for.
    project: Option<ProjectId>,
    /// Document revision at the last autosave version (or when the project was loaded).
    last_revision: u64,
    /// When the last autosave version was written (or the project was loaded).
    last_version_ms: u64,
    /// This controller's session id (marker contents), drawn on first use.
    session: Option<String>,
    /// Projects whose marker from another session was still there when they were loaded.
    stale_on_open: BTreeSet<ProjectId>,
}

fn kind_tag(kind: VersionKind) -> &'static str {
    match kind {
        VersionKind::Autosave => "autosave",
        VersionKind::Manual => "manual",
        VersionKind::BeforeRestore => "before-restore",
    }
}

/// `"<created ms>-<kind>"` → (created ms, kind).
pub fn parse_id(id: &str) -> Option<(u64, VersionKind)> {
    let (ms, kind) = id.split_once('-')?;
    if ms.is_empty() || !ms.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let kind = match kind {
        "autosave" => VersionKind::Autosave,
        "manual" => VersionKind::Manual,
        "before-restore" => VersionKind::BeforeRestore,
        _ => return None,
    };
    Some((ms.parse().ok()?, kind))
}

fn version_path(id: &str) -> CmdResult<String> {
    if parse_id(id).is_none() {
        return Err(invalid(format!("not a version id: {id:?}")));
    }
    Ok(format!("{VERSIONS_DIR}/{id}{EXT}"))
}

fn clean_name(name: Option<&str>) -> Option<String> {
    let name = name?.trim();
    (!name.is_empty()).then(|| name.chars().take(MAX_NAME_CHARS).collect())
}

/// Versions of project `id` in its store folder, newest first.
fn list_versions<S: ProjectStore>(store: &mut S, id: ProjectId) -> CmdResult<Vec<VersionInfo>> {
    let listing = match store.list_dir(id, VERSIONS_DIR) {
        Ok(l) => l,
        Err(StoreError::NotFound(_)) => return Ok(Vec::new()),
        Err(e) => return Err(store_err(e)),
    };
    let names = read_names(store, id);
    let mut out: Vec<VersionInfo> = listing
        .entries
        .into_iter()
        .filter_map(|e| {
            let vid = e.name.strip_suffix(EXT)?;
            let (created_ms, kind) = parse_id(vid)?;
            Some(VersionInfo {
                id: vid.to_string(),
                kind,
                name: names.get(vid).cloned(),
                created_ms,
                size: e.size.max(0.0) as u64,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then(b.id.cmp(&a.id)));
    Ok(out)
}

fn read_names<S: ProjectStore>(store: &mut S, id: ProjectId) -> BTreeMap<String, String> {
    store
        .read(id, NAMES_FILE)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_names<S: ProjectStore>(
    store: &mut S,
    id: ProjectId,
    names: &BTreeMap<String, String>,
) -> CmdResult<()> {
    let json = serde_json::to_vec_pretty(names)
        .map_err(|e| cmd_err(ErrorCode::Io, format!("version names: {e}")))?;
    store.write(id, NAMES_FILE, &json).map_err(store_err)
}

/// Marker contents: `{ "session": "<id>" }`.
fn marker_session(bytes: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    v.get("session")?.as_str().map(str::to_owned)
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn version_command(
        &mut self,
        command: &VersionCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            VersionCommand::List => {
                let id = self.open_project_id()?;
                Ok(ReplyValue::Versions {
                    versions: list_versions(&mut self.store, id)?,
                })
            }
            VersionCommand::Create { name } => {
                let version =
                    self.write_version(VersionKind::Manual, clean_name(name.as_deref()), now)?;
                self.emit_versions_changed(out);
                Ok(ReplyValue::Version { version })
            }
            VersionCommand::Restore { version } => {
                self.restore_version(version, now, out)?;
                Ok(ReplyValue::Unit)
            }
            VersionCommand::Delete { version } => {
                let id = self.open_project_id()?;
                let path = version_path(version)?;
                self.store.read(id, &path).map_err(store_err)?;
                self.store.remove(id, &path).map_err(store_err)?;
                let mut names = read_names(&mut self.store, id);
                if names.remove(version).is_some() {
                    write_names(&mut self.store, id, &names)?;
                }
                self.emit_versions_changed(out);
                Ok(ReplyValue::Unit)
            }
            VersionCommand::Rename { version, name } => {
                let id = self.open_project_id()?;
                let path = version_path(version)?;
                self.store.read(id, &path).map_err(store_err)?;
                let mut names = read_names(&mut self.store, id);
                match clean_name(name.as_deref()) {
                    Some(n) => names.insert(version.clone(), n),
                    None => names.remove(version),
                };
                write_names(&mut self.store, id, &names)?;
                self.emit_versions_changed(out);
                Ok(ReplyValue::Unit)
            }
            VersionCommand::Compare { version, against } => {
                let id = self.open_project_id()?;
                let base = self.read_version(id, version)?;
                let target = match against {
                    Some(v) => self.read_version(id, v)?,
                    None => {
                        // Like-for-like with the stored versions (live plugin states).
                        let doc = self.doc.as_ref().ok_or_else(no_project)?.project.clone();
                        let json = self.serialize(&doc)?;
                        file::load(&json)
                            .map_err(|e| cmd_err(ErrorCode::Io, format!("project file: {e}")))?
                    }
                };
                Ok(ReplyValue::VersionDiff {
                    diff: diff(&base, &target),
                })
            }
            VersionCommand::ListRecoverable => Ok(ReplyValue::Recoverable {
                projects: self.list_recoverable()?,
            }),
            VersionCommand::Recover { project } => self.recover(*project, now, out),
            VersionCommand::DiscardRecovery { project } => {
                self.versions.stale_on_open.remove(project);
                let ours = self.doc.as_ref().is_some_and(|d| d.project.id == *project);
                if !ours {
                    self.store
                        .remove(*project, SESSION_MARKER)
                        .map_err(store_err)?;
                }
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Called every tick: a rolling autosave version when due.
    pub(crate) fn versions_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(id) = self.doc.as_ref().map(|d| d.project.id) else {
            return;
        };
        if self.versions.project != Some(id)
            || self.revision == self.versions.last_revision
            || now.saturating_sub(self.versions.last_version_ms) < VERSION_INTERVAL_MS
        {
            return;
        }
        // Whatever happens, don't retry before the next interval.
        self.versions.last_revision = self.revision;
        self.versions.last_version_ms = now;
        match self.write_version(VersionKind::Autosave, None, now) {
            Ok(_) => {
                self.prune_autosaves(id);
                self.emit_versions_changed(out);
            }
            Err(e) => notify(
                out,
                NotificationLevel::Warning,
                format!("saving a project version failed: {}", e.message),
            ),
        }
    }

    /// A project became the open document (create, open, restore, recover): start its
    /// session (marker, version clock). Loading another project closes the previous
    /// session cleanly.
    pub(crate) fn versions_on_load(&mut self, now: u64) {
        let Some(id) = self.doc.as_ref().map(|d| d.project.id) else {
            return;
        };
        if let Some(prev) = self.versions.project
            && prev != id
        {
            let _ = self.store.remove(prev, SESSION_MARKER);
            self.versions.stale_on_open.remove(&prev);
        }
        self.versions.last_revision = self.revision;
        self.versions.last_version_ms = now;
        if self.versions.project == Some(id) {
            return;
        }
        self.versions.project = Some(id);
        let session = self.session_id();
        if let Ok(bytes) = self.store.read(id, SESSION_MARKER)
            && marker_session(&bytes).as_deref() != Some(session.as_str())
        {
            self.versions.stale_on_open.insert(id);
        }
        let marker = serde_json::json!({ "session": session, "opened_ms": now }).to_string();
        let _ = self.store.write(id, SESSION_MARKER, marker.as_bytes());
    }

    /// The open document moved to another project folder (Save As): the old folder's
    /// session is closed, the new one's starts.
    pub(crate) fn versions_on_identity_change(&mut self, now: u64) {
        self.versions_on_load(now);
    }

    /// A copied project folder (Duplicate, Save As) must not carry the source's marker.
    pub(crate) fn versions_forget_marker(&mut self, id: ProjectId) {
        let _ = self.store.remove(id, SESSION_MARKER);
    }

    /// Clean close: remove the open project's marker.
    pub(crate) fn versions_close(&mut self) {
        if let Some(id) = self.versions.project.take() {
            let _ = self.store.remove(id, SESSION_MARKER);
        }
    }

    fn session_id(&mut self) -> String {
        if self.versions.session.is_none() {
            let seed = self.host.random_seed();
            let now = self.host.now_ms();
            self.versions.session = Some(format!("{now:x}-{seed:016x}"));
        }
        self.versions.session.clone().expect("just set")
    }

    fn open_project_id(&self) -> CmdResult<ProjectId> {
        Ok(self.doc.as_ref().ok_or_else(no_project)?.project.id)
    }

    fn emit_versions_changed(&self, out: &mut dyn MessageSink) {
        event(
            out,
            Event::Version {
                event: VersionEvent::Changed,
            },
        );
    }

    fn read_version(
        &mut self,
        id: ProjectId,
        version: &str,
    ) -> CmdResult<ether_core::protocol::model::Project> {
        let path = version_path(version)?;
        let bytes = self.store.read(id, &path).map_err(store_err)?;
        let json = String::from_utf8(bytes)
            .map_err(|e| cmd_err(ErrorCode::Io, format!("version {version}: {e}")))?;
        let mut project = file::load(&json)
            .map_err(|e| cmd_err(ErrorCode::Io, format!("version {version}: {e}")))?;
        project.id = id;
        Ok(project)
    }

    /// Snapshot the open document as a new version (unique id, named if `name`).
    fn write_version(
        &mut self,
        kind: VersionKind,
        name: Option<String>,
        now: u64,
    ) -> CmdResult<VersionInfo> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let (id, project) = (doc.project.id, doc.project.clone());
        let json = self.serialize(&project)?;
        let existing: BTreeSet<String> = list_versions(&mut self.store, id)?
            .into_iter()
            .map(|v| v.id)
            .collect();
        let mut created_ms = now;
        let mut vid = format!("{created_ms}-{}", kind_tag(kind));
        while existing.contains(&vid) {
            created_ms += 1;
            vid = format!("{created_ms}-{}", kind_tag(kind));
        }
        self.store
            .write(id, &format!("{VERSIONS_DIR}/{vid}{EXT}"), json.as_bytes())
            .map_err(store_err)?;
        if let Some(n) = &name {
            let mut names = read_names(&mut self.store, id);
            names.insert(vid.clone(), n.clone());
            write_names(&mut self.store, id, &names)?;
        }
        Ok(VersionInfo {
            id: vid,
            kind,
            name,
            created_ms,
            size: json.len() as u64,
        })
    }

    /// Keep the newest `MAX_AUTOSAVE_VERSIONS` autosave versions.
    fn prune_autosaves(&mut self, id: ProjectId) {
        let Ok(versions) = list_versions(&mut self.store, id) else {
            return;
        };
        let old: Vec<VersionId> = versions
            .into_iter()
            .filter(|v| v.kind == VersionKind::Autosave)
            .skip(MAX_AUTOSAVE_VERSIONS)
            .map(|v| v.id)
            .collect();
        if old.is_empty() {
            return;
        }
        let mut names = read_names(&mut self.store, id);
        let mut renamed = false;
        for v in &old {
            let _ = self.store.remove(id, &format!("{VERSIONS_DIR}/{v}{EXT}"));
            renamed |= names.remove(v).is_some();
        }
        if renamed {
            let _ = write_names(&mut self.store, id, &names);
        }
    }

    fn restore_version(
        &mut self,
        version: &str,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let id = self.open_project_id()?;
        if self.collab_active() {
            return Err(invalid_state(
                "leave the collaboration session before restoring a version",
            ));
        }
        // Validate before touching anything.
        let project = self.read_version(id, version)?;
        self.write_version(VersionKind::BeforeRestore, None, now)?;
        self.load_project(project, now, out);
        self.set_dirty(true, out);
        self.emit_versions_changed(out);
        Ok(())
    }

    fn list_recoverable(&mut self) -> CmdResult<Vec<RecoveryInfo>> {
        let session = self.session_id();
        let open = self.doc.as_ref().map(|d| d.project.id);
        let mut out = Vec::new();
        for summary in self.store.list().map_err(store_err)? {
            let stale = if Some(summary.id) == open {
                self.versions.stale_on_open.contains(&summary.id)
            } else {
                self.store
                    .read(summary.id, SESSION_MARKER)
                    .is_ok_and(|b| marker_session(&b).as_deref() != Some(session.as_str()))
            };
            if !stale {
                continue;
            }
            let Some(newest) = list_versions(&mut self.store, summary.id)
                .ok()
                .and_then(|v| v.into_iter().next())
            else {
                continue;
            };
            let saved_ms = summary.modified_ms.max(0.0) as u64;
            if newest.created_ms > saved_ms && self.differs_from_saved(summary.id, &newest.id) {
                out.push(RecoveryInfo {
                    project: summary.id,
                    name: summary.name,
                    version: newest,
                    saved_ms,
                });
            }
        }
        Ok(out)
    }

    /// The version holds something `project.ether` doesn't (a version saved right before a
    /// clean save, then a web tab closed without a clean close, is nothing to recover).
    /// Unreadable documents count as different.
    fn differs_from_saved(&mut self, id: ProjectId, version: &str) -> bool {
        let Ok(recovered) = self.read_version(id, version) else {
            return false;
        };
        let Some(mut saved) = self
            .store
            .load(id)
            .ok()
            .and_then(|json| file::load(&json).ok())
        else {
            return true;
        };
        saved.id = id;
        let d = diff(&saved, &recovered);
        d.settings_changed || !d.tables.is_empty()
    }

    fn recover(
        &mut self,
        project: ProjectId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let newest = list_versions(&mut self.store, project)?
            .into_iter()
            .next()
            .ok_or_else(|| invalid(format!("project {project} has no versions to recover")))?;
        let recovered = self.read_version(project, &newest.id)?;
        if self.collab_active() {
            return Err(invalid_state(
                "leave the collaboration session before recovering a project",
            ));
        }
        let open = self.doc.as_ref().is_some_and(|d| d.project.id == project);
        if !open {
            // Saves the open project (if dirty) and closes its session.
            self.autosave_before_switch(out)?;
            // The stale marker is ours now: `versions_on_load` overwrites it.
            let _ = self.store.remove(project, SESSION_MARKER);
        }
        self.versions.stale_on_open.remove(&project);
        self.load_project(recovered.clone(), now, out);
        self.set_dirty(true, out);
        Ok(ReplyValue::Project {
            project: Box::new(recovered),
        })
    }
}

impl<B, H, S, L> Drop for EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// A clean close removes the session marker; a panic leaves it for crash recovery.
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.versions_close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for kind in [
            VersionKind::Autosave,
            VersionKind::Manual,
            VersionKind::BeforeRestore,
        ] {
            let id = format!("1750000000000-{}", kind_tag(kind));
            assert_eq!(parse_id(&id), Some((1_750_000_000_000, kind)));
        }
        for bad in ["", "-manual", "12-", "12-other", "x1-manual", "../1-manual"] {
            assert_eq!(parse_id(bad), None, "{bad}");
        }
        assert!(version_path("../../project").is_err());
    }

    #[test]
    fn names_are_trimmed_and_capped() {
        assert_eq!(clean_name(Some("  ")), None);
        assert_eq!(clean_name(Some(" Mix 2 ")).as_deref(), Some("Mix 2"));
        assert_eq!(
            clean_name(Some(&"x".repeat(500))).map(|n| n.len()),
            Some(MAX_NAME_CHARS)
        );
    }
}
