//! Media preview voice (roadmap, owned by the `media-preview` node; see `docs/ROADMAP.md`
//! and CONTRACTS.md §11.15).
//!
//! Auditions a sample from the browser: one voice, outside the tracks, mixed straight into
//! the hardware outputs after master (not metered, not recorded, not exported, independent
//! of the transport: it plays while stopped).
//!
//! Wired (base-24):
//! - `EngineHandle::preview(PreviewControl)` sends play/stop through the control ring. Each
//!   `Play` carries a controller-chosen preview `id` (monotonic). The source is an ordinary
//!   [`AudioSource`] (decoded and resampled to the engine rate by the controller, like clip
//!   media), so the audio thread never allocates or frees it (a replaced, stopped or
//!   finished source is retired to the GC).
//! - `engine.rs::render_sub` calls [`PreviewVoice::render`] once per sub-block.
//! - **End reporting (natural ends only).** When the voice reaches the end of its source,
//!   the engine reports that preview's id through `EngineOutputs::preview_ended` (the most
//!   recent one if several ended between two polls). Stop and replace are *not* reported:
//!   the controller knows about them (it sent them) and emits `PreviewEnded { Stopped |
//!   Replaced }` itself; it emits `PreviewEnded { Finished }` only if the reported id is
//!   still its current preview. So every preview gets exactly one `PreviewEnded`.
//!
//! Implemented: play from the start at `gain`, mono sources on both channels, auto-stop at
//! the end of the source. **Left to the media-preview node:** a short fade on stop/replace
//! (today it cuts), optional looping, and everything outside the engine (controller decode
//! + `EngineBridge` plumbing, `MediaEvent`s, UI).

use std::sync::Arc;

use crate::media::AudioSource;

/// Preview control message (`EngineHandle::preview`).
pub enum PreviewControl {
    /// Start `source` from its beginning (replacing any playing preview, which is not
    /// reported) at linear `gain`. `id` identifies this preview in
    /// `EngineOutputs::preview_ended`.
    Play {
        id: u64,
        source: Arc<dyn AudioSource>,
        gain: f32,
    },
    /// Stop the playing preview, if any (not reported).
    Stop,
}

impl std::fmt::Debug for PreviewControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Play { id, gain, .. } => f
                .debug_struct("Play")
                .field("id", id)
                .field("gain", gain)
                .finish(),
            Self::Stop => f.write_str("Stop"),
        }
    }
}

/// The audio-thread preview voice.
pub(crate) struct PreviewVoice {
    source: Option<Arc<dyn AudioSource>>,
    id: u64,
    gain: f32,
    position: u64,
    scratch: [Vec<f32>; 2],
    /// Id of the latest preview that reached its end (natural end), not yet reported.
    pub(crate) ended: Option<u64>,
    /// A source that finished while rendering, to retire after the block.
    retired: Option<Arc<dyn AudioSource>>,
}

impl PreviewVoice {
    /// Non-RT.
    pub(crate) fn new(max_block_size: usize) -> Self {
        Self {
            source: None,
            id: 0,
            gain: 1.0,
            position: 0,
            scratch: [vec![0.0; max_block_size], vec![0.0; max_block_size]],
            ended: None,
            retired: None,
        }
    }

    /// RT. Apply a control; returns the source that must be retired (never dropped here).
    pub(crate) fn control(&mut self, control: PreviewControl) -> Option<Arc<dyn AudioSource>> {
        let old = self.source.take();
        if let PreviewControl::Play { id, source, gain } = control {
            self.source = Some(source);
            self.id = id;
            self.gain = gain.max(0.0);
            self.position = 0;
        }
        old
    }

    /// RT. The source that finished during the last renders, to retire.
    pub(crate) fn take_retired(&mut self) -> Option<Arc<dyn AudioSource>> {
        self.retired.take()
    }

    /// RT. Mix `frames` samples into `out[ch][offset..]` (hardware outputs; clamped).
    pub(crate) fn render(&mut self, offset: usize, frames: usize, out: &mut [&mut [f32]]) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let n = frames.min(self.scratch[0].len());
        let channels = source.channels().max(1);
        for ch in 0..2u16 {
            let src_ch = ch.min(channels - 1);
            source.read(src_ch, self.position, &mut self.scratch[ch as usize][..n]);
        }
        source.prefetch_hint(self.position + n as u64);
        for (ch, o) in out.iter_mut().take(2).enumerate() {
            let end = (offset + n).min(o.len());
            if offset >= end {
                continue;
            }
            for (d, s) in o[offset..end].iter_mut().zip(&self.scratch[ch][..n]) {
                *d += s * self.gain;
            }
        }
        self.position += n as u64;
        if self.position >= source.frames() {
            self.ended = Some(self.id);
            self.retired = self.source.take();
        }
    }
}
