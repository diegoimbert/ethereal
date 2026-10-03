//! Streaming long audio from disk (v0.3, owned by the `audio-streaming` node; CONTRACTS.md
//! §13.1).
//!
//! Today every media is decoded whole and resampled by the controller (`media/`), then
//! handed to the engine (`EngineBridge::load_media`): about 115 MB for 5 minutes of stereo.
//! With streaming, media longer than [`STREAM_MIN_SECONDS`] on a host that can stream
//! (`EngineBridge::stream_media` returns `Ok(true)`) are **not** decoded into memory:
//! - the controller still runs one decode pass for the waveform peaks (kept, the samples
//!   dropped) and the content hash, then calls `stream_media` with a [`StreamSource`];
//! - the host registers an `AudioSource` whose `read` serves from a bounded read-ahead
//!   cache (fixed-size chunks at the engine rate, decoded and resampled on a host reader
//!   thread: native = a disk thread, web = the engine Worker over OPFS sync access handles,
//!   chunks shipped to the worklet over the SAB ring). The audio thread never blocks,
//!   allocates or does I/O: a miss is an underrun (silence, `read` returns `false`, counted
//!   and reported);
//! - `AudioSource::prefetch_hint` (called by the engine for upcoming clip reads) and the
//!   controller's loop/locate knowledge drive the read-ahead, so loops and jumps don't
//!   underrun; warp/stretch keep working because the stretcher reads through the same
//!   `AudioSource` (it reads ahead by its own latency, inside the cache window);
//! - offline renders (export, freeze, bounce, audio-to-MIDI) read the file synchronously
//!   (they may block).
//!
//! [`should_stream`] is the policy hook the `media/` pipeline calls: media of known length
//! at least [`STREAM_MIN_SECONDS`] long. Hosts that can't stream (`stream_media` returns
//! `Ok(false)`, the default) or fail to (`Err`) get the whole-file decode as before.
//!
//! Underruns: the engine counts reads that returned `false` (`EngineOutputs::underruns`);
//! while media are streamed, [`EtherController::stream_tick`] reports new ones as
//! `MediaEvent::StreamUnderruns`, at most every [`UNDERRUN_REPORT_MS`].

use ether_core::protocol::Event;
use ether_core::protocol::media::MediaEvent;
use ether_core::protocol::model::{MediaRef, ProjectId};

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Minimum interval between two `MediaEvent::StreamUnderruns`.
pub const UNDERRUN_REPORT_MS: u64 = 1000;

/// Media at least this long (at their own rate) stream instead of being decoded whole.
pub const STREAM_MIN_SECONDS: f64 = 30.0;
/// Read-ahead window per streamed media voice, in seconds at the engine rate.
pub const STREAM_READ_AHEAD_SECONDS: f64 = 4.0;

/// What the host needs to stream one media (plain data, so the web bridge can ship it to
/// the Worker).
#[derive(Clone, Debug, PartialEq)]
pub struct StreamSource {
    pub project: ProjectId,
    /// The media (id, project-relative `file`, `location` for references, rate, frames).
    pub media: MediaRef,
    /// Resolved external path when the media is referenced in place and its hash matched.
    pub external_path: Option<String>,
    /// Rate the engine runs at (the source resamples to it).
    pub engine_sample_rate: u32,
}

/// Policy: stream `media`? Yes when its length is known (`frames > 0`; media imported
/// without a length are decoded whole, which learns it) and at least
/// [`STREAM_MIN_SECONDS`]. Shorter media (one-shots, loops) stay in memory: instant random
/// access for samplers and no disk traffic.
pub fn should_stream(media: &MediaRef, engine_sample_rate: u32) -> bool {
    engine_sample_rate > 0
        && media.sample_rate > 0
        && media.frames > 0
        && media.frames as f64 / media.sample_rate as f64 >= STREAM_MIN_SECONDS
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Called every tick after the engine outputs were polled: report new underruns while
    /// media are streamed.
    pub(crate) fn stream_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let media = &mut self.media;
        if !media.any_streamed() {
            media.underruns_pending = 0;
            return;
        }
        // `EngineOutputs::underruns` counts since the previous poll.
        media.underruns_pending = media
            .underruns_pending
            .saturating_add(self.outputs.underruns);
        if media.underruns_pending == 0
            || now.saturating_sub(media.underruns_at) < UNDERRUN_REPORT_MS
        {
            return;
        }
        let count = std::mem::take(&mut media.underruns_pending);
        media.underruns_at = now;
        event(
            out,
            Event::Media {
                event: MediaEvent::StreamUnderruns { count },
            },
        );
    }
}
