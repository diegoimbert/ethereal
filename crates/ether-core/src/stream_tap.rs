//! Stream tap (base-53; docs/COLLAB.md §9.1): a copy of what the engine renders **after
//! master, the metronome and the count-in, and before the browser preview voice**, for
//! "listen on <peer>" (the host streams it to its listeners over WebRTC). Previews stay
//! private to each site.
//!
//! The host allocates a ring pair with [`stream_tap_ring`] (off the audio thread) and hands
//! the writer to the engine with `EngineHandle::set_stream_tap(Some(..))`; the reader goes
//! to its sender thread. While a writer is installed, every rendered sub-block pushes one
//! [`StreamBlock`] (transport state at the block start) and then `frames` interleaved
//! stereo samples. RT-safe: no allocation, no locks. A sub-block that does not fit (reader
//! too slow) is skipped whole and the next written block has `gap = true`.
//! `set_stream_tap(None)` removes the writer (it is retired to the GC, never dropped on the
//! audio thread).

use rtrb::{Consumer, Producer, RingBuffer};

use crate::transport::TransportInfo;

/// Transport state of one tapped sub-block (at its first frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamBlock {
    /// Engine sample clock of the first frame (monotonic, never reset by locate).
    pub sample_time: u64,
    /// Stereo frames that follow in the audio ring.
    pub frames: u32,
    /// Timeline position (beats) at the first frame, before latency compensation.
    pub position: f64,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f64,
    /// Output latency (samples, PDC) of the published graph: the timeline rendered at
    /// `sample_time` (at `position`) reaches the tap `latency` samples later; the audio of
    /// this block is the timeline rendered `latency` samples earlier (docs/COLLAB.md §9.4).
    pub latency: u32,
    /// The timeline jumped to `position` at this block's first frame: playback started
    /// from stopped, a locate, or a loop wrap. Set on the first sub-block after the jump
    /// only (carried to the next written block if that one was skipped), and on the first
    /// block after the tap is installed.
    pub jump: bool,
    /// Blocks were skipped right before this one (ring full).
    pub gap: bool,
}

/// The engine side of a tap (installed with `EngineHandle::set_stream_tap`).
pub struct StreamTapWriter {
    pub audio: Producer<f32>,
    pub blocks: Producer<StreamBlock>,
}

/// The sender side of a tap.
pub struct StreamTapReader {
    pub audio: Consumer<f32>,
    pub blocks: Consumer<StreamBlock>,
}

/// A tap ring holding `frames` stereo frames (and as many block headers as sub-blocks of
/// 16 frames would need). Not RT: call it on a host thread.
pub fn stream_tap_ring(frames: usize) -> (StreamTapWriter, StreamTapReader) {
    let (ap, ac) = RingBuffer::new(frames.max(1) * 2);
    let (bp, bc) = RingBuffer::new((frames / 16).max(64));
    (
        StreamTapWriter {
            audio: ap,
            blocks: bp,
        },
        StreamTapReader {
            audio: ac,
            blocks: bc,
        },
    )
}

/// Engine-side state.
#[derive(Default)]
pub(crate) struct StreamTap {
    writer: Option<Box<StreamTapWriter>>,
    gap: bool,
    /// A jump not yet reported (its block was skipped).
    jump: bool,
}

impl StreamTap {
    /// Install or remove the writer; returns the previous one (to retire).
    pub(crate) fn set(
        &mut self,
        writer: Option<Box<StreamTapWriter>>,
    ) -> Option<Box<StreamTapWriter>> {
        self.gap = false;
        // A fresh reader has no timeline yet: its first block starts one.
        self.jump = true;
        std::mem::replace(&mut self.writer, writer)
    }

    /// **RT.** Copy `outputs[..2][off..off + n]` (mono is duplicated, missing channels are
    /// silent) with the block's transport state. `jump`: the timeline jumped to
    /// `info.position` at this sub-block (kept for the next written block if this one is
    /// skipped).
    pub(crate) fn write(
        &mut self,
        info: &TransportInfo,
        latency: u32,
        jump: bool,
        off: usize,
        n: usize,
        outputs: &[&mut [f32]],
    ) {
        let Some(w) = self.writer.as_mut() else {
            return;
        };
        self.jump |= jump;
        if n == 0 {
            return;
        }
        let Ok(chunk) = w.audio.write_chunk_uninit(2 * n) else {
            self.gap = true;
            return;
        };
        if w.blocks.slots() < 1 {
            self.gap = true;
            return; // the unused chunk is dropped without committing
        }
        let block = StreamBlock {
            sample_time: info.sample_time,
            frames: n as u32,
            position: info.position,
            playing: info.playing,
            recording: info.recording,
            bpm: info.bpm,
            latency,
            jump: std::mem::take(&mut self.jump),
            gap: std::mem::take(&mut self.gap),
        };
        let _ = w.blocks.push(block);
        let silent: &[f32] = &[];
        let left: &[f32] = outputs.first().map_or(silent, |o| &o[..]);
        let right: &[f32] = outputs.get(1).map_or(left, |o| &o[..]);
        let at = |ch: &[f32], i: usize| ch.get(off + i).copied().unwrap_or(0.0);
        chunk.fill_from_iter((0..2 * n).map(|k| {
            let i = k / 2;
            if k % 2 == 0 {
                at(left, i)
            } else {
                at(right, i)
            }
        }));
    }
}
