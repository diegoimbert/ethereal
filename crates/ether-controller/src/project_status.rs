//! Archived projects and the bin (`Project::{SetArchived, MoveToBin, RestoreFromBin,
//! ListBin, EmptyBin}`).
//!
//! A project's status is a small file in its folder ([`STATUS_FILE`], written through the
//! generic `ProjectStore::{read, write, remove}`, so native disk, web OPFS and the memory
//! store all work): `{ "archived": true }`, `{ "binned_ms": <when> }`. No file = a live
//! project. Copies (`Duplicate`, `SaveAs`) never carry it.
//!
//! - `List` (and `ListChanged`) leaves the projects in the bin out and flags the archived
//!   ones ([`ProjectSummary::archived`]); `ListBin` lists the bin.
//! - Every listing deletes the projects that have been in the bin for
//!   [`BIN_RETENTION_MS`] (the bin empties itself after 30 days).
//! - A project in the bin can't be opened, nor be the open project (`MoveToBin` refuses it,
//!   as `Delete` does).

use serde::{Deserialize, Serialize};

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::{BIN_RETENTION_MS, ProjectSummary};

use crate::handlers::store_err;
use crate::store::{Library, ProjectStore, StoreError};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// The status file, project-relative (hidden: not shown with the project's files).
pub const STATUS_FILE: &str = ".status.json";

/// What [`STATUS_FILE`] holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// In the bin since then (Unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binned_ms: Option<u64>,
}

impl Status {
    fn is_live(&self) -> bool {
        *self == Self::default()
    }
}

/// The status of project `id` (no file, or an unreadable one: live).
pub fn read_status<S: ProjectStore>(store: &mut S, id: ProjectId) -> Status {
    store
        .read(id, STATUS_FILE)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Write project `id`'s status (a live one removes the file).
pub fn write_status<S: ProjectStore>(
    store: &mut S,
    id: ProjectId,
    status: &Status,
) -> Result<(), StoreError> {
    if status.is_live() {
        return store.remove(id, STATUS_FILE);
    }
    let json = serde_json::to_vec(status).map_err(|e| StoreError::Io(e.to_string()))?;
    store.write(id, STATUS_FILE, &json)
}

/// Whether a project binned at `binned_ms` is due for deletion at `now`.
pub fn expired(binned_ms: u64, now: u64) -> bool {
    now.saturating_sub(binned_ms) >= BIN_RETENTION_MS
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// The stored projects split into the live ones (archived flagged) and the bin (newest
    /// binned first), the expired ones of the bin deleted on the way.
    pub(crate) fn stored_projects(
        &mut self,
    ) -> CmdResult<(Vec<ProjectSummary>, Vec<ProjectSummary>)> {
        let now = self.host.now_ms();
        let open = self.doc.as_ref().map(|d| d.project.id);
        let mut live = Vec::new();
        let mut bin = Vec::new();
        for mut s in self.store.list().map_err(store_err)? {
            let status = read_status(&mut self.store, s.id);
            match status.binned_ms {
                // Never the open project (it can't be binned; a stale file is ignored).
                Some(at) if Some(s.id) != open => {
                    if expired(at, now) {
                        match self.store.delete(s.id) {
                            Ok(()) => {
                                tracing::info!(project = %s.id, "deleted from the bin after 30 days")
                            }
                            Err(e) => {
                                tracing::warn!(project = %s.id, error = %e, "could not empty the bin")
                            }
                        }
                        continue;
                    }
                    s.binned_ms = Some(at as f64);
                    s.archived = status.archived;
                    bin.push(s);
                }
                _ => {
                    s.archived = status.archived;
                    live.push(s);
                }
            }
        }
        bin.sort_by(|a, b| {
            b.binned_ms
                .unwrap_or(0.0)
                .total_cmp(&a.binned_ms.unwrap_or(0.0))
        });
        Ok((live, bin))
    }

    /// Refuse `id` when it is in the bin (opening it).
    pub(crate) fn check_not_binned(&mut self, id: ProjectId) -> CmdResult<()> {
        if read_status(&mut self.store, id).binned_ms.is_some() {
            return Err(crate::tx::keyed_err(
                ether_core::protocol::ErrorCode::InvalidState,
                crate::text::msg("error.project.in_bin"),
            ));
        }
        Ok(())
    }

    /// `MoveToBin`, `RestoreFromBin`, `SetArchived`: change `id`'s status.
    pub(crate) fn set_status(
        &mut self,
        id: ProjectId,
        change: impl FnOnce(&mut Status),
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        // A stored project (its folder is there).
        self.store.list_dir(id, "").map_err(store_err)?;
        let mut status = read_status(&mut self.store, id);
        change(&mut status);
        write_status(&mut self.store, id, &status).map_err(store_err)?;
        self.emit_list_changed(out);
        Ok(ReplyValue::Unit)
    }

    /// `EmptyBin`: delete every project in the bin.
    pub(crate) fn empty_bin(&mut self, out: &mut dyn MessageSink) -> CmdResult<ReplyValue> {
        let (_, bin) = self.stored_projects()?;
        for s in bin {
            self.store.delete(s.id).map_err(store_err)?;
        }
        self.emit_list_changed(out);
        Ok(ReplyValue::Unit)
    }
}
