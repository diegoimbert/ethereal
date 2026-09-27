//! Media preview voice (roadmap, owned by the `media-preview` node; see `docs/ROADMAP.md`).
//!
//! Auditions a sample from the browser: one voice, outside the tracks, mixed straight into
//! the hardware outputs after master (not metered, not recorded, not exported, independent
//! of the transport: it plays while stopped).
//!
//! Wired (base-24):
//! - `EngineHandle::preview(PreviewControl)` sends play/stop through the control ring; the
//!   source is an ordinary [`AudioSource`] (decoded and resampled to the engine rate by the
//!   controller, like clip media), so the audio thread never allocates or frees it (a
//!   replaced or finished source is retired to the GC).
//! - `engine.rs::render_sub` calls [`PreviewVoice::render`] once per sub-block.
//! - When the voice reaches the end of its source (auto-stop) or is stopped/replaced, the
//!   engine reports it once through `EngineOutputs::preview_ended`, which the controller
//!   turns into `MediaEvent::PreviewEnded`.
//!
//! Implemented: play from the start at `gain`, mono sources on both channels, auto-stop at
//! the end of the source. **Left to the media-preview node:** a short fade on stop/replace
//! (today it cuts), optional looping/sync-to-tempo, and everything outside the engine
//! (controller decode + `EngineBridge` plumbing, `MediaEvent`s, UI).

use std::sync::Arc;

use crate::media::AudioSource;

/// Preview control message (`EngineHandle::preview`).
pub enum PreviewControl {
    /// Start `source` from its beginning (replacing any playing preview) at linear `gain`.
    Play {
        source: Arc<dyn AudioSource>,
        gain: f32,
    },
    Stop,
}

impl std::fmt::Debug for PreviewControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Play { gain, .. } => f.debug_struct("Play").field("gain", gain).finish(),
            Self::Stop => f.write_str("Stop"),
        }
    }
}

/// The audio-thread preview voice.
pub(crate) struct PreviewVoice {
    source: Option<Arc<dyn AudioSource>>,
    gain: f32,
    position: u64,
    scratch: [Vec<f32>; 2],
    /// Set when a preview ends (end reached, stopped or replaced); reported once.
    pub(crate) ended: bool,
    /// A source that finished while rendering, to retire after the block.
    retired: Option<Arc<dyn AudioSource>>,
}

impl PreviewVoice {
    /// Non-RT.
    pub(crate) fn new(max_block_size: usize) -> Self {
        Self {
            source: None,
            gain: 1.0,
            position: 0,
            scratch: [vec![0.0; max_block_size], vec![0.0; max_block_size]],
            ended: false,
            retired: None,
        }
    }

    /// RT. Apply a control; returns the source that must be retired (never dropped here).
    pub(crate) fn control(&mut self, control: PreviewControl) -> Option<Arc<dyn AudioSource>> {
        let old = self.source.take();
        if old.is_some() {
            self.ended = true;
        }
        if let PreviewControl::Play { source, gain } = control {
            self.source = Some(source);
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
            self.ended = true;
            self.retired = self.source.take();
        }
    }
}
