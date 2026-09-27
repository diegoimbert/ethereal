//! Media preview voice (roadmap, owned by the `media-preview` node; see `docs/ROADMAP.md`
//! and CONTRACTS.md §11.15).
//!
//! Auditions a sample from the browser: one voice, outside the tracks, mixed straight into
//! the hardware outputs after master (not metered, not recorded, not exported, independent
//! of the transport: it plays while stopped).
//!
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
//! Play starts from the beginning at `gain`, mono sources on both channels, and stops at
//! the end of the source. Stop and replace fade the old preview out linearly over
//! [`FADE_FRAMES`] (~5 ms) so cutting a sample never clicks; on replace the new preview
//! starts at once (its attack is kept) while the old one fades out under it. The fading
//! source lives in a second slot and is retired to the GC when its fade ends.

use std::sync::Arc;

use crate::media::AudioSource;

/// Preview control message (`EngineHandle::preview`).
pub enum PreviewControl {
    /// Start `source` from its beginning (replacing any playing preview, which fades out
    /// and is not reported) at linear `gain`. `id` identifies this preview in
    /// `EngineOutputs::preview_ended`.
    Play {
        id: u64,
        source: Arc<dyn AudioSource>,
        gain: f32,
    },
    /// Stop the playing preview, if any (fades out; not reported).
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

/// Length of the fade-out of a stopped or replaced preview, in frames (5 ms at 48 kHz; the
/// voice doesn't know the engine rate, so it is a fixed frame count).
pub const FADE_FRAMES: u32 = 240;

/// Sources waiting to be handed to the GC (`take_retired`, once per engine block). A render
/// retires at most two (natural end, end of a fade); if the slots are ever full, a source
/// stays (silent) in its voice slot until one frees up.
const RETIRE_SLOTS: usize = 4;

/// The audio-thread preview voice.
pub(crate) struct PreviewVoice {
    source: Option<Arc<dyn AudioSource>>,
    id: u64,
    gain: f32,
    position: u64,
    /// The stopped/replaced preview, fading out (`fade_left` of `FADE_FRAMES` frames left).
    fading: Option<Arc<dyn AudioSource>>,
    fade_gain: f32,
    fade_position: u64,
    fade_left: u32,
    scratch: [Vec<f32>; 2],
    /// Id of the latest preview that reached its end (natural end), not yet reported.
    pub(crate) ended: Option<u64>,
    /// Sources that finished while rendering, to retire after the block.
    retired: [Option<Arc<dyn AudioSource>>; RETIRE_SLOTS],
}

impl PreviewVoice {
    /// Non-RT.
    pub(crate) fn new(max_block_size: usize) -> Self {
        Self {
            source: None,
            id: 0,
            gain: 1.0,
            position: 0,
            fading: None,
            fade_gain: 0.0,
            fade_position: 0,
            fade_left: 0,
            scratch: [vec![0.0; max_block_size], vec![0.0; max_block_size]],
            ended: None,
            retired: Default::default(),
        }
    }

    /// RT. Apply a control; returns the source that must be retired (never dropped here):
    /// one still fading from an earlier stop/replace, if any. The playing preview (if any)
    /// starts fading out.
    pub(crate) fn control(&mut self, control: PreviewControl) -> Option<Arc<dyn AudioSource>> {
        let evicted = match self.source.take() {
            Some(old) if self.position < old.frames() => {
                self.fade_gain = self.gain;
                self.fade_position = self.position;
                self.fade_left = FADE_FRAMES;
                self.fading.replace(old)
            }
            // At its end, waiting for a retire slot: nothing to fade.
            Some(old) => Some(old),
            None => None,
        };
        if let PreviewControl::Play { id, source, gain } = control {
            self.source = Some(source);
            self.id = id;
            self.gain = gain.max(0.0);
            self.position = 0;
        }
        evicted
    }

    /// RT. A source that finished during the last renders, to retire.
    pub(crate) fn take_retired(&mut self) -> Option<Arc<dyn AudioSource>> {
        self.retired.iter_mut().find_map(Option::take)
    }

    /// RT. Queue `source` for retirement; gives it back if every slot is taken.
    fn stash(&mut self, source: Arc<dyn AudioSource>) -> Option<Arc<dyn AudioSource>> {
        match self.retired.iter_mut().find(|s| s.is_none()) {
            Some(slot) => {
                *slot = Some(source);
                None
            }
            None => Some(source),
        }
    }

    /// RT. Mix `frames` samples into `out[ch][offset..]` (hardware outputs; clamped).
    pub(crate) fn render(&mut self, offset: usize, frames: usize, out: &mut [&mut [f32]]) {
        let n = frames.min(self.scratch[0].len());
        self.render_fade(offset, n, out);
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let total = source.frames();
        if self.position < total {
            read_stereo(source.as_ref(), self.position, &mut self.scratch, n);
            source.prefetch_hint(self.position + n as u64);
            let gain = self.gain;
            mix(out, offset, n, &self.scratch, |_| gain);
            self.position += n as u64;
            if self.position >= total {
                self.ended = Some(self.id);
            }
        }
        if self.position >= total
            && let Some(s) = self.source.take()
        {
            self.source = self.stash(s);
        }
    }

    /// RT. The fading (stopped/replaced) source: a linear ramp from its gain down to 0.
    fn render_fade(&mut self, offset: usize, n: usize, out: &mut [&mut [f32]]) {
        let Some(source) = self.fading.as_ref() else {
            return;
        };
        let total = source.frames();
        if self.fade_left > 0 && self.fade_position < total {
            let m = n.min(self.fade_left as usize);
            read_stereo(source.as_ref(), self.fade_position, &mut self.scratch, m);
            let step = self.fade_gain / FADE_FRAMES as f32;
            let start = step * self.fade_left as f32;
            mix(out, offset, m, &self.scratch, |i| {
                (start - step * (i + 1) as f32).max(0.0)
            });
            self.fade_position += m as u64;
            self.fade_left -= m as u32;
        }
        if (self.fade_left == 0 || self.fade_position >= total)
            && let Some(s) = self.fading.take()
        {
            self.fade_left = 0;
            self.fading = self.stash(s);
        }
    }
}

/// RT. Read `n` frames of `source` at `position` into both scratch channels (mono on both).
fn read_stereo(source: &dyn AudioSource, position: u64, scratch: &mut [Vec<f32>; 2], n: usize) {
    let channels = source.channels().max(1);
    for ch in 0..2u16 {
        let src_ch = ch.min(channels - 1);
        source.read(src_ch, position, &mut scratch[ch as usize][..n]);
    }
}

/// RT. `out[ch][offset + i] += scratch[ch][i] * gain(i)` for `i < n` (clamped to `out`).
fn mix(
    out: &mut [&mut [f32]],
    offset: usize,
    n: usize,
    scratch: &[Vec<f32>; 2],
    gain: impl Fn(usize) -> f32,
) {
    for (ch, o) in out.iter_mut().take(2).enumerate() {
        let end = (offset + n).min(o.len());
        if offset >= end {
            continue;
        }
        for (i, (d, s)) in o[offset..end].iter_mut().zip(&scratch[ch][..n]).enumerate() {
            *d += s * gain(i);
        }
    }
}
