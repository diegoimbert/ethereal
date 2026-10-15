//! The background writer behind [`DiskStore`]'s `save_in_background`: routine autosaves
//! are serialized and written (atomically, fsynced) on their own thread, so a big project
//! doesn't block the controller thread (and the play/stop commands queued behind it).
//!
//! - One job at a time, in order. A queued job that hasn't started is replaced by a newer
//!   one for the same project: only the latest document matters.
//! - [`SaveWriter::flush`] waits for the queue to drain; the store calls it before every
//!   other access to a project document, so reads and synchronous saves never see (or
//!   get overwritten by) an older write.
//! - Dropping the last [`DiskStore`] clone finishes the queued work, then joins the thread
//!   (quitting never loses an autosave that was already accepted).
//!
//! [`DiskStore`]: crate::store::DiskStore

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Instant;

use ether_controller::store::{RenderDocument, StoreError};
use ether_core::protocol::model::ProjectId;

use crate::store::atomic_write;

struct Job {
    id: ProjectId,
    path: PathBuf,
    render: RenderDocument,
}

#[derive(Default)]
struct State {
    queued: Option<Job>,
    /// A job is being rendered/written.
    busy: bool,
    stop: bool,
    /// The first failure not reported yet (`take_error`).
    error: Option<(ProjectId, StoreError)>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking job is caught in `run`; the state itself stays consistent.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed.wait(guard).unwrap_or_else(|p| p.into_inner())
    }
}

/// Shared by every clone of a `DiskStore` (the thread starts with the first job).
#[derive(Default)]
pub(crate) struct SaveWriter {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for SaveWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.shared.lock();
        f.debug_struct("SaveWriter")
            .field("queued", &s.queued.as_ref().map(|j| j.id))
            .field("busy", &s.busy)
            .finish()
    }
}

impl SaveWriter {
    /// Queue `render` to be written to `path` (`project.ether` of `id`).
    pub(crate) fn submit(&self, id: ProjectId, path: PathBuf, render: RenderDocument) {
        let mut s = self.shared.lock();
        // Another project's queued document is never dropped: wait until it started.
        while s.queued.as_ref().is_some_and(|j| j.id != id) {
            s = self.shared.wait(s);
        }
        if s.queued.is_some() {
            tracing::debug!(project = %id, "queued autosave replaced by a newer one");
        }
        s.queued = Some(Job { id, path, render });
        drop(s);
        self.shared.changed.notify_all();
        let mut thread = self.thread.lock().unwrap_or_else(|p| p.into_inner());
        if thread.is_none() {
            let shared = self.shared.clone();
            match std::thread::Builder::new()
                .name("ether-save".into())
                .spawn(move || run(&shared))
            {
                Ok(h) => *thread = Some(h),
                Err(e) => {
                    // No thread: write here (the caller is blocked, as before).
                    tracing::warn!(error = %e, "could not start the save thread");
                    if let Some(job) = self.shared.lock().queued.take() {
                        let result = write(job.path, job.render);
                        if let Err(e) = result {
                            self.shared.lock().error.get_or_insert((job.id, e));
                        }
                    }
                }
            }
        }
    }

    /// Wait until nothing is queued or being written.
    pub(crate) fn flush(&self) {
        let mut s = self.shared.lock();
        while s.queued.is_some() || s.busy {
            s = self.shared.wait(s);
        }
    }

    pub(crate) fn take_error(&self) -> Option<(ProjectId, StoreError)> {
        self.shared.lock().error.take()
    }
}

impl Drop for SaveWriter {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        let thread = self
            .thread
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(h) = thread {
            let _ = h.join();
        }
        if let Some((id, e)) = self.shared.lock().error.take() {
            tracing::error!(project = %id, error = %e, "background save failed");
        }
    }
}

fn run(shared: &Shared) {
    loop {
        let job = {
            let mut s = shared.lock();
            loop {
                if let Some(job) = s.queued.take() {
                    s.busy = true;
                    break job;
                }
                if s.stop {
                    return;
                }
                s = shared.wait(s);
            }
        };
        // Wakes a `submit` waiting for another project's job to start.
        shared.changed.notify_all();
        let id = job.id;
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write(job.path, job.render)))
                .unwrap_or_else(|_| Err(StoreError::Io("saving the project panicked".into())));
        let mut s = shared.lock();
        s.busy = false;
        if let Err(e) = result {
            tracing::error!(project = %id, error = %e, "background save failed");
            s.error.get_or_insert((id, e));
        }
        drop(s);
        shared.changed.notify_all();
    }
}

fn write(path: PathBuf, render: RenderDocument) -> Result<(), StoreError> {
    let started = Instant::now();
    let json = render()?;
    let render_ms = started.elapsed().as_millis() as u64;
    atomic_write(&path, json.as_bytes())?;
    tracing::info!(
        path = %path.display(),
        kb = json.len() / 1024,
        render_ms,
        write_ms = started.elapsed().as_millis() as u64 - render_ms,
        "project saved (background)"
    );
    Ok(())
}
