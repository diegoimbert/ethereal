//! Audio sources for audio clips and samplers.

/// Read access to (decoded, resampled-to-engine-rate) audio. Implemented by hosts:
/// fully-loaded buffers (web, short files), or disk-streaming sources whose `read` serves
/// from a prefetch ring filled by a host thread (native).
///
/// `read` runs on the audio thread: it must not block, allocate or do I/O. If data isn't
/// available yet it fills silence and returns `false` (an underrun the host can report).
///
/// Sources are shared via `Arc`; the engine never drops the last reference on the audio
/// thread (retired snapshots are dropped by the `GarbageCollector`).
pub trait AudioSource: Send + Sync {
    fn channels(&self) -> u16;
    /// Length in frames at the engine sample rate.
    fn frames(&self) -> u64;
    /// Copy frames `[start, start + out.len())` of `channel` into `out` (zero past the end).
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool;
    /// Hint from the engine about upcoming reads (streaming sources prefetch). RT-safe.
    fn prefetch_hint(&self, start: u64) {
        let _ = start;
    }
}
