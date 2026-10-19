//! What a save reads from the plugins, and when plugin state changes alone get saved.
//!
//! Reading a plugin's state (`EngineBridge::plugin_state`: `getChunk`/`getState` on the
//! plugin's main thread) can be slow, big projects have a hundred plugins or more, and
//! plugins report `StateDirty` very often (right after loading, while their GUI is open,
//! for internal changes such as a meter's history). Saving the whole project on each of
//! those, reading every plugin each time, blocked the controller thread for seconds every
//! few seconds. So:
//!
//! - `StateDirty` is not a document edit: it marks the device (its state is re-read by the
//!   next save) and leaves `DirtyChanged` alone, so the UI's quick autosave after edits
//!   doesn't fire for it. Unsaved plugin state still counts as unsaved work (quit, project
//!   switch: [`PluginStates::has_unsaved`]) and is written by the next save of any kind, or
//!   by the controller's slow timer ([`PluginStates::save_due`]).
//! - Each save goes through a cache of the last state read from every instance.
//!   [`SaveKind::Routine`] reads only the instances that reported a change, were (re)created
//!   or were never read; [`SaveKind::Full`] reads them all.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::model::{Base64Bytes, DeviceId};

/// Why a save happens, which decides how plugin states are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    /// Autosaves (the UI's timer after edits, the controller's timers, rolling versions):
    /// re-read only the plugins that reported a change since they were last read, use the
    /// cache for the others; the file may be written in the background.
    Routine,
    /// The user saves (Ctrl+S, the agent), or the project is about to be closed, switched
    /// away from, copied or exported: every plugin's live state is read and the file is
    /// written before returning. Correctness first: some formats (VST2 especially) don't
    /// reliably report state changes, so these never trust the cache.
    Full,
}

/// Plugin state changes are saved on their own at most this often (the oldest unsaved
/// change is at least this old)...
pub(crate) const STATE_SAVE_INTERVAL_MS: u64 = 30_000;
/// ...once no plugin reported a change for this long...
pub(crate) const STATE_SAVE_IDLE_MS: u64 = 2_000;
/// ...or anyway once the oldest unsaved change is this old (a plugin that reports changes
/// non-stop would otherwise never be saved by the timer).
pub(crate) const STATE_SAVE_MAX_DELAY_MS: u64 = 120_000;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    /// What the instance returned (`None`: no state; the document's is kept).
    state: Option<Base64Bytes>,
    /// Included in the last save of the open project.
    written: bool,
}

/// Per-device cache of plugin states and the unsaved `StateDirty` bookkeeping (runtime
/// state of the open project; reset when another one loads).
#[derive(Debug, Default)]
pub(crate) struct PluginStates {
    read: BTreeMap<DeviceId, Entry>,
    /// Must be read again before the next save: reported `StateDirty`, or its instance
    /// was re-created.
    stale: BTreeSet<DeviceId>,
    /// First and last `StateDirty` not saved yet.
    unsaved: Option<(u64, u64)>,
    /// A background write failed: the next routine save writes synchronously, so a
    /// persisting error is reported in its reply instead of re-queueing writes.
    pub(crate) write_failed: bool,
    /// The plugins the last `read_plugin_states` could not read (their last known state
    /// was used), with the error.
    pub(crate) read_failures: Vec<(DeviceId, String)>,
}

impl PluginStates {
    /// Another project is open.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// The plugin reported `StateDirty` at `now`.
    pub(crate) fn state_changed(&mut self, device: DeviceId, now: u64) {
        self.stale.insert(device);
        let first = self.unsaved.map_or(now, |(first, _)| first);
        self.unsaved = Some((first, now));
    }

    /// The device's instance was (re)created, or its state may differ from the cache for
    /// another reason: read it again before the next save.
    pub(crate) fn invalidate(&mut self, device: DeviceId) {
        self.stale.insert(device);
    }

    pub(crate) fn needs_read(&self, device: DeviceId, kind: SaveKind) -> bool {
        kind == SaveKind::Full || self.stale.contains(&device) || !self.read.contains_key(&device)
    }

    /// A state was read from the live instance.
    pub(crate) fn record(&mut self, device: DeviceId, state: Option<Base64Bytes>) {
        self.stale.remove(&device);
        let written = self
            .read
            .get(&device)
            .is_some_and(|e| e.written && e.state == state);
        self.read.insert(device, Entry { state, written });
    }

    pub(crate) fn cached(&self, device: DeviceId) -> Option<&Base64Bytes> {
        self.read.get(&device)?.state.as_ref()
    }

    /// Forget devices that are gone.
    pub(crate) fn retain(&mut self, keep: impl Fn(DeviceId) -> bool) {
        self.read.retain(|d, _| keep(*d));
        self.stale.retain(|d| keep(*d));
    }

    /// Some state read since the last save differs from what that save wrote (or was
    /// never written).
    pub(crate) fn has_unwritten(&self) -> bool {
        self.read.values().any(|e| !e.written)
    }

    /// A plugin reported a state change that no save has written yet.
    pub(crate) fn has_unsaved(&self) -> bool {
        self.unsaved.is_some()
    }

    /// The open project was saved with the cached states.
    pub(crate) fn saved(&mut self) {
        for e in self.read.values_mut() {
            e.written = true;
        }
        self.unsaved = None;
    }

    /// Nothing to write after all (the re-read states are the saved ones).
    pub(crate) fn nothing_unsaved(&mut self) {
        self.unsaved = None;
    }

    /// The timer's save failed: try again after another interval.
    pub(crate) fn postpone(&mut self, now: u64) {
        if let Some((_, last)) = self.unsaved {
            self.unsaved = Some((now, last));
        }
    }

    /// Unsaved plugin state changes are due to be saved on their own.
    pub(crate) fn save_due(&self, now: u64) -> bool {
        self.unsaved.is_some_and(|(first, last)| {
            let age = now.saturating_sub(first);
            age >= STATE_SAVE_INTERVAL_MS
                && (now.saturating_sub(last) >= STATE_SAVE_IDLE_MS
                    || age >= STATE_SAVE_MAX_DELAY_MS)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::IdGen;

    #[test]
    fn timer_waits_for_the_interval_and_a_pause() {
        let d: DeviceId = IdGen::new(1).next(0);
        let mut s = PluginStates::default();
        assert!(!s.save_due(1_000_000));
        s.state_changed(d, 0);
        assert!(!s.save_due(STATE_SAVE_INTERVAL_MS - 1));
        assert!(s.save_due(STATE_SAVE_INTERVAL_MS));
        // Still reporting changes: wait for a pause...
        s.state_changed(d, STATE_SAVE_INTERVAL_MS);
        assert!(!s.save_due(STATE_SAVE_INTERVAL_MS + 1));
        assert!(s.save_due(STATE_SAVE_INTERVAL_MS + STATE_SAVE_IDLE_MS));
        // ...but not forever.
        s.state_changed(d, STATE_SAVE_MAX_DELAY_MS - 1);
        assert!(s.save_due(STATE_SAVE_MAX_DELAY_MS));
        s.saved();
        assert!(!s.has_unsaved() && !s.save_due(u64::MAX));
    }

    #[test]
    fn reads_are_written_until_they_change() {
        let d: DeviceId = IdGen::new(1).next(0);
        let mut s = PluginStates::default();
        assert!(s.needs_read(d, SaveKind::Routine));
        s.record(d, Some(Base64Bytes(vec![1])));
        assert!(!s.needs_read(d, SaveKind::Routine));
        assert!(s.needs_read(d, SaveKind::Full));
        assert!(s.has_unwritten());
        s.saved();
        assert!(!s.has_unwritten());
        s.state_changed(d, 5);
        assert!(s.needs_read(d, SaveKind::Routine));
        // Same bytes again: still written.
        s.record(d, Some(Base64Bytes(vec![1])));
        assert!(!s.has_unwritten());
        s.record(d, Some(Base64Bytes(vec![2])));
        assert!(s.has_unwritten());
        assert_eq!(s.cached(d), Some(&Base64Bytes(vec![2])));
    }
}
