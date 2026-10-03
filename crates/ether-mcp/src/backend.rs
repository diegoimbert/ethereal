//! The engine an MCP session drives.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ether_protocol::{Command, ReplyResult, ReplyValue};

/// How long a request may take before the caller gives up (exports run in the background,
/// so no tool call should come close).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// An Ethereal engine reachable with request/reply commands. Blocking; called from a
/// worker thread.
pub trait Backend: Send + Sync {
    /// Send one command and wait for its reply. `Err` = transport failure or command error,
    /// as a message for the model.
    fn request(&self, command: Command) -> Result<ReplyValue, String>;

    /// Called after a successful `save_project` tool call; returns a note for the model
    /// (e.g. where the file was written).
    fn after_save(&self) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// One line describing the engine (for the MCP `instructions` and logs).
    fn describe(&self) -> String;
}

/// Request ids and the callers waiting for their replies.
#[derive(Default)]
pub struct Pending {
    next: AtomicU32,
    waiting: Mutex<HashMap<u32, std::sync::mpsc::Sender<Result<ReplyResult, String>>>>,
}

impl Pending {
    /// A fresh request id and the receiver of its reply.
    pub fn register(&self) -> (u32, std::sync::mpsc::Receiver<Result<ReplyResult, String>>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let (tx, rx) = std::sync::mpsc::channel();
        self.lock().insert(id, tx);
        (id, rx)
    }

    /// Deliver the reply `id` (ignored if nobody waits).
    pub fn complete(&self, id: u32, result: Result<ReplyResult, String>) {
        if let Some(tx) = self.lock().remove(&id) {
            let _ = tx.send(result);
        }
    }

    pub fn forget(&self, id: u32) {
        self.lock().remove(&id);
    }

    /// Fail every waiting request (connection lost).
    pub fn fail_all(&self, message: &str) {
        for (_, tx) in self.lock().drain() {
            let _ = tx.send(Err(message.to_string()));
        }
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u32, std::sync::mpsc::Sender<Result<ReplyResult, String>>>>
    {
        self.waiting.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Wait for a reply registered with [`Pending::register`].
pub fn wait(
    pending: &Pending,
    id: u32,
    rx: std::sync::mpsc::Receiver<Result<ReplyResult, String>>,
) -> Result<ReplyValue, String> {
    match rx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(Ok(ReplyResult::Ok { value })) => Ok(value),
        Ok(Ok(ReplyResult::Err { error })) => Err(error.message),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            pending.forget(id);
            Err(format!(
                "Ethereal did not answer within {} s",
                REQUEST_TIMEOUT.as_secs()
            ))
        }
    }
}
