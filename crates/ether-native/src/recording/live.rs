//! Live recording view (`live-record`): waveform peaks and MIDI notes of the takes being
//! recorded, handed from the writer thread to the controller thread.
//!
//! - The writer thread computes min/max peaks (all channels of a take file merged, fixed
//!   [`FRAMES_PER_PEAK`]) while it writes the WAV files, so the live peaks cover exactly the
//!   kept frames of each take (count-in and punch excluded) and match the file's peaks.
//! - Hand-off: a [`LiveQueue`] behind a mutex that only the writer thread and the controller
//!   thread (`EngineBridge::poll_recording`) take; the audio thread never touches it. It is
//!   bounded: when the controller stalls the oldest entries are dropped (and counted).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use ether_core::protocol::model::TrackId;
use ether_core::protocol::recording::{LiveAudioChunk, LiveMidiNote};

/// Input frames per live peak.
pub(crate) const FRAMES_PER_PEAK: u32 = 256;
/// Queued audio chunks kept when the controller does not poll (~10 s of one track at the
/// writer's 5 ms poll).
pub(super) const MAX_CHUNKS: usize = 2048;
/// Queued MIDI notes kept when the controller does not poll.
pub(super) const MAX_NOTES: usize = 4096;
/// Shortest live note (beats), like the committed notes.
const MIN_LENGTH: f64 = 1.0 / 64.0;

/// Live data not yet polled by the controller.
#[derive(Debug, Default)]
pub(crate) struct LiveQueue {
    audio: VecDeque<LiveAudioChunk>,
    midi: VecDeque<LiveMidiNote>,
    /// Entries dropped because the queue was full.
    dropped: u64,
}

impl LiveQueue {
    pub(super) fn push_audio(&mut self, c: LiveAudioChunk) {
        if self.audio.len() >= MAX_CHUNKS {
            self.audio.pop_front();
            self.dropped += 1;
        }
        self.audio.push_back(c);
    }

    pub(super) fn push_midi(&mut self, n: LiveMidiNote) {
        if self.midi.len() >= MAX_NOTES {
            self.midi.pop_front();
            self.dropped += 1;
        }
        self.midi.push_back(n);
    }

    pub(super) fn drain(&mut self, audio: &mut Vec<LiveAudioChunk>, midi: &mut Vec<LiveMidiNote>) {
        audio.extend(self.audio.drain(..));
        midi.extend(self.midi.drain(..));
    }

    pub(super) fn clear(&mut self) {
        self.audio.clear();
        self.midi.clear();
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Entries dropped since the last call (reported at the end of each session).
    pub(super) fn take_dropped(&mut self) -> u64 {
        std::mem::take(&mut self.dropped)
    }
}

/// Shared between the writer thread and the bridge (controller thread) only.
#[derive(Clone, Debug, Default)]
pub(crate) struct LiveShared(Arc<Mutex<LiveQueue>>);

impl LiveShared {
    pub(crate) fn lock(&self) -> MutexGuard<'_, LiveQueue> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Incremental min/max peaks of one take file.
#[derive(Debug)]
pub(super) struct PeakAcc {
    track: TrackId,
    take: u32,
    start: f64,
    sample_rate: u32,
    /// Extremes of the peak being accumulated, over `n` frames.
    lo: f32,
    hi: f32,
    n: u32,
    /// Index of the first pending peak.
    first: u64,
    min: Vec<f32>,
    max: Vec<f32>,
}

impl PeakAcc {
    pub fn new(track: TrackId, take: u32, start: f64, sample_rate: u32) -> Self {
        Self {
            track,
            take,
            start,
            sample_rate,
            lo: f32::INFINITY,
            hi: f32::NEG_INFINITY,
            n: 0,
            first: 0,
            min: Vec::new(),
            max: Vec::new(),
        }
    }

    /// One sample of the current frame (any channel).
    #[inline]
    pub fn sample(&mut self, s: f32) {
        self.lo = self.lo.min(s);
        self.hi = self.hi.max(s);
    }

    /// The current frame is complete.
    #[inline]
    pub fn end_frame(&mut self) {
        self.n += 1;
        if self.n == FRAMES_PER_PEAK {
            self.close_peak();
        }
    }

    fn close_peak(&mut self) {
        if self.n == 0 {
            return;
        }
        let (lo, hi) = if self.lo <= self.hi {
            (self.lo, self.hi)
        } else {
            (0.0, 0.0)
        };
        self.min.push(lo);
        self.max.push(hi);
        self.lo = f32::INFINITY;
        self.hi = f32::NEG_INFINITY;
        self.n = 0;
    }

    /// Queue the completed peaks (`last`: the take ended, include the partial peak).
    pub fn flush(&mut self, live: &mut LiveQueue, last: bool) {
        if last {
            self.close_peak();
        }
        if self.min.is_empty() {
            return;
        }
        let count = self.min.len() as u64;
        live.push_audio(LiveAudioChunk {
            track: self.track,
            take: self.take,
            start: self.start,
            sample_rate: self.sample_rate,
            frames_per_peak: FRAMES_PER_PEAK,
            first_peak: self.first,
            min: std::mem::take(&mut self.min),
            max: std::mem::take(&mut self.max),
        });
        self.first += count;
    }
}

/// Pairs note on/off of the recorded MIDI into live notes, exactly like the controller's
/// `notes_from_midi` pairs the committed ones. The host does not know which MIDI tracks are
/// armed: notes carry [`TrackId::NIL`] and the controller fans them out to its armed MIDI
/// tracks.
#[derive(Debug, Default)]
pub(super) struct LiveNotes {
    /// (channel, key, start, velocity).
    held: Vec<(u8, u8, f64, u8)>,
}

impl LiveNotes {
    /// A kept MIDI message at its latency-compensated `position`.
    pub fn message(&mut self, data: [u8; 3], position: f64, live: &mut LiveQueue) {
        let status = data[0] & 0xf0;
        let channel = data[0] & 0x0f;
        let key = data[1] & 0x7f;
        let vel = data[2] & 0x7f;
        let on = status == 0x90 && vel > 0;
        let off = status == 0x80 || (status == 0x90 && vel == 0);
        if !(on || off) {
            return;
        }
        if let Some(i) = self.held.iter().position(|h| h.0 == channel && h.1 == key) {
            let (_, _, t0, v) = self.held.remove(i);
            live.push_midi(LiveMidiNote {
                track: TrackId::NIL,
                pitch: key,
                velocity: v,
                start: t0,
                length: Some((position - t0).max(MIN_LENGTH)),
            });
        }
        if on {
            self.held.push((channel, key, position, vel));
            live.push_midi(LiveMidiNote {
                track: TrackId::NIL,
                pitch: key,
                velocity: vel,
                start: position,
                length: None,
            });
        }
    }
}
