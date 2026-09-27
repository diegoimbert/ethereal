//! Uploads from the UI machine (roadmap v2, owned by the `remote-engine` node; see
//! `docs/ROADMAP.md` and `ether_protocol::media` / `ether_protocol::remote`).
//!
//! `Media::{BeginUpload, UploadChunk, CancelUpload}` land here: bytes are staged in the
//! store (`ProjectStore::{begin, append, read, discard}_upload`; natively a file under
//! `<projects_root>/.uploads/`, never kept in controller memory), then
//! `Media::Import { source: Upload }` imports the completed upload like a library file
//! ([`take_upload`]) and drops the staging.
//!
//! Rules:
//! - `BeginUpload { upload, name, size }`: `size` in `1..=MAX_UPLOAD_BYTES`, `name` a
//!   non-empty display name (its extension picks the decoder), at most
//!   [`MAX_ACTIVE_UPLOADS`] at once. Reusing an id restarts that upload.
//! - `UploadChunk { offset, data }`: at most [`MAX_CHUNK_BYTES`]. `offset` must equal the
//!   bytes received so far; a chunk that only repeats bytes already received is accepted
//!   as a no-op (a client retrying after a lost reply). A gap is `InvalidState` whose
//!   message names the expected offset, so the client can resume from there.
//!   `MediaEvent::UploadProgress` is emitted at most every [`PROGRESS_INTERVAL_MS`] and
//!   always when the upload completes.
//! - `CancelUpload`: drops the staging (unknown ids are fine). Hosts cancel a
//!   client's uploads when it disconnects.
//! - Uploads idle for [`UPLOAD_IDLE_MS`] are discarded on the next upload command.

use std::collections::BTreeMap;

use ether_core::protocol::ReplyValue;
use ether_core::protocol::media::{MediaCommand, MediaEvent};
use ether_core::protocol::message::Event;

use crate::handlers::{event, store_err};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Largest accepted upload (1 GiB).
pub const MAX_UPLOAD_BYTES: u64 = 1 << 30;
/// Largest accepted chunk (1 MiB, see `ether_protocol::media`).
pub const MAX_CHUNK_BYTES: usize = 1 << 20;
/// Concurrent uploads per controller.
pub const MAX_ACTIVE_UPLOADS: usize = 16;
/// An upload without a chunk for this long is abandoned.
pub const UPLOAD_IDLE_MS: u64 = 10 * 60 * 1000;
/// Minimum interval between two `UploadProgress` events of one upload.
pub const PROGRESS_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone)]
struct Upload {
    name: String,
    size: u64,
    received: u64,
    last_activity_ms: u64,
    last_progress_ms: Option<u64>,
}

/// Uploads in progress (metadata only; the bytes are in the store).
#[derive(Debug, Default)]
pub(crate) struct UploadState {
    uploads: BTreeMap<String, Upload>,
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `BeginUpload` / `UploadChunk` / `CancelUpload`.
    pub(crate) fn upload_command(
        &mut self,
        c: &MediaCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let now = self.host.now_ms();
        self.discard_idle_uploads(now);
        match c {
            MediaCommand::BeginUpload { upload, name, size } => {
                if !size.is_finite() || *size < 1.0 || *size > MAX_UPLOAD_BYTES as f64 {
                    return Err(invalid(format!(
                        "upload size must be 1..={MAX_UPLOAD_BYTES} bytes"
                    )));
                }
                // A display name, never a path.
                let name = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
                if name.is_empty() {
                    return Err(invalid("upload name is empty"));
                }
                let restarting = self.uploads.uploads.contains_key(upload);
                if !restarting && self.uploads.uploads.len() >= MAX_ACTIVE_UPLOADS {
                    return Err(invalid_state(format!(
                        "too many uploads in progress (max {MAX_ACTIVE_UPLOADS})"
                    )));
                }
                let size = *size as u64;
                self.store.begin_upload(upload, size).map_err(store_err)?;
                self.uploads.uploads.insert(
                    upload.clone(),
                    Upload {
                        name: name.to_string(),
                        size,
                        received: 0,
                        last_activity_ms: now,
                        last_progress_ms: None,
                    },
                );
                Ok(ReplyValue::Unit)
            }
            MediaCommand::UploadChunk {
                upload,
                offset,
                data,
            } => {
                let u = self
                    .uploads
                    .uploads
                    .get(upload)
                    .ok_or_else(|| not_found(format!("upload {upload}")))?;
                let bytes = &data.0;
                if bytes.len() > MAX_CHUNK_BYTES {
                    return Err(invalid(format!(
                        "upload chunks are at most {MAX_CHUNK_BYTES} bytes"
                    )));
                }
                if !offset.is_finite() || *offset < 0.0 || offset.fract() != 0.0 {
                    return Err(invalid("bad upload offset"));
                }
                let offset = *offset as u64;
                let end = offset + bytes.len() as u64;
                if offset < u.received && end <= u.received {
                    // A retried chunk we already have: nothing to do.
                    return Ok(ReplyValue::Unit);
                }
                if offset != u.received {
                    return Err(invalid_state(format!(
                        "upload {upload}: expected offset {}, got {offset}",
                        u.received
                    )));
                }
                if end > u.size {
                    return Err(invalid(format!(
                        "upload {upload}: {end} bytes exceed the announced {}",
                        u.size
                    )));
                }
                let received = self
                    .store
                    .append_upload(upload, offset, bytes)
                    .map_err(store_err)?;
                let u = self.uploads.uploads.get_mut(upload).expect("checked above");
                u.received = received;
                u.last_activity_ms = now;
                let complete = received == u.size;
                let due = u
                    .last_progress_ms
                    .is_none_or(|t| now.saturating_sub(t) >= PROGRESS_INTERVAL_MS);
                if complete || due {
                    u.last_progress_ms = Some(now);
                    event(
                        out,
                        Event::Media {
                            event: MediaEvent::UploadProgress {
                                upload: upload.clone(),
                                received: received as f64,
                            },
                        },
                    );
                }
                Ok(ReplyValue::Unit)
            }
            MediaCommand::CancelUpload { upload } => {
                self.uploads.uploads.remove(upload);
                self.store.discard_upload(upload).map_err(store_err)?;
                Ok(ReplyValue::Unit)
            }
            _ => Err(invalid("not an upload command")),
        }
    }

    fn discard_idle_uploads(&mut self, now: u64) {
        let idle: Vec<String> = self
            .uploads
            .uploads
            .iter()
            .filter(|(_, u)| now.saturating_sub(u.last_activity_ms) >= UPLOAD_IDLE_MS)
            .map(|(id, _)| id.clone())
            .collect();
        for id in idle {
            self.uploads.uploads.remove(&id);
            let _ = self.store.discard_upload(&id);
        }
    }
}

/// The bytes and file name of a completed upload, for `Import { source: Upload }` (a free
/// function so `import` can call it while it borrows the document). The upload is consumed
/// (its staging dropped) whether or not the import then succeeds. `NotFound` for unknown
/// ids, `InvalidState` while incomplete.
pub(crate) fn take_upload<S: ProjectStore>(
    state: &mut UploadState,
    store: &mut S,
    upload: &str,
) -> CmdResult<(Vec<u8>, String)> {
    let u = state
        .uploads
        .get(upload)
        .ok_or_else(|| not_found(format!("upload {upload}")))?;
    if u.received != u.size {
        return Err(invalid_state(format!(
            "upload {upload} is incomplete ({} of {} bytes)",
            u.received, u.size
        )));
    }
    let name = u.name.clone();
    let bytes = store.read_upload(upload).map_err(store_err)?;
    state.uploads.remove(upload);
    let _ = store.discard_upload(upload);
    Ok((bytes, name))
}
