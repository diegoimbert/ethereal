//! The audio device's period, as the real-time callback sees it.
//!
//! Nodes that wait on something outside the engine (the plugin sandbox's helper process)
//! must bound that wait by what the *device* allows, not by the engine's max block size:
//! at a 128-frame period a 1024-frame budget is eight periods. The host's audio callback
//! calls [`begin_callback`] once per device buffer; it publishes the buffer's length and a
//! wall-clock deadline for external waits that **all** such nodes share, so N late nodes
//! wait at most one budget per buffer in total, not N budgets.
//!
//! Process-wide (one audio device per process), and only seen from the threads that
//! render device buffers: the callback thread itself (marked by [`begin_callback`]) and
//! the engine's worker threads ([`mark_device_thread`]). An offline render (export) on
//! another thread, even while the device runs, sees no deadline and its nodes fall back
//! to their own budget, as do tests without a callback.
//!
//! RT-safe: atomics and `Instant::now` (vDSO clock read) only.

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Share of a device buffer that external waits may use, from the callback's start.
/// Engine DSP needs the rest.
pub const EXTERNAL_WAIT_SHARE: f64 = 0.5;

static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Frames of the last device buffer (0 = no callback yet).
static PERIOD_FRAMES: AtomicU32 = AtomicU32::new(0);
/// Deadline of external waits in the current buffer, ns since [`EPOCH`] (0 = none).
static DEADLINE_NS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// This thread renders device buffers (callback or engine worker).
    static DEVICE_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Mark the current thread as one that renders device buffers (the engine's worker
/// threads, at start), so its nodes see [`wait_deadline`].
pub fn mark_device_thread() {
    DEVICE_THREAD.with(|d| d.set(true));
}

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

fn ns_since_epoch(t: Instant) -> u64 {
    t.saturating_duration_since(epoch()).as_nanos() as u64
}

/// **RT.** Called by the audio callback at the start of every device buffer of `frames`
/// frames at `sample_rate`.
pub fn begin_callback(frames: usize, sample_rate: f64) {
    mark_device_thread();
    let now = Instant::now();
    let period = frames as f64 / sample_rate.max(1.0);
    let budget = Duration::from_secs_f64(period * EXTERNAL_WAIT_SHARE);
    PERIOD_FRAMES.store(frames.min(u32::MAX as usize) as u32, Ordering::Relaxed);
    // Never 0 (0 = "no deadline").
    DEADLINE_NS.store(ns_since_epoch(now + budget).max(1), Ordering::Release);
}

/// The last device buffer's length in frames, if a callback ran (sizes a node's latency
/// FIFO at activation).
pub fn device_period() -> Option<usize> {
    match PERIOD_FRAMES.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n as usize),
    }
}

/// **RT.** The shared deadline of external waits in the current device buffer, if the
/// audio callback published one and this thread renders device buffers.
pub fn wait_deadline() -> Option<Instant> {
    if !DEVICE_THREAD.with(Cell::get) {
        return None;
    }
    match DEADLINE_NS.load(Ordering::Acquire) {
        0 => None,
        ns => Some(epoch() + Duration::from_nanos(ns)),
    }
}

/// Forget the published period (tests; a host whose stream stopped).
pub fn reset() {
    PERIOD_FRAMES.store(0, Ordering::Relaxed);
    DEADLINE_NS.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_publishes_period_and_a_shared_deadline() {
        reset();
        assert_eq!(device_period(), None);
        assert!(wait_deadline().is_none());
        let before = Instant::now();
        begin_callback(256, 48_000.0);
        assert_eq!(device_period(), Some(256));
        let d = wait_deadline().unwrap();
        // Half of 5.33 ms from now.
        let budget = d.saturating_duration_since(before);
        assert!(
            budget >= Duration::from_micros(2_600) && budget <= Duration::from_millis(50),
            "{budget:?}"
        );
        // Another thread (an offline render) never sees the device's deadline.
        assert!(std::thread::spawn(wait_deadline).join().unwrap().is_none());
        let worker = std::thread::spawn(|| {
            mark_device_thread();
            wait_deadline()
        });
        assert_eq!(worker.join().unwrap(), Some(d));
        reset();
    }
}
