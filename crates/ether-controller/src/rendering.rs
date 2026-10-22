//! A render suspends the live engine and plays its preview (CONTRACTS.md §13.20).
//!
//! An audio export (mix, stems, both; the private overlay's share snapshots render through
//! it too) renders on an offline engine of its own, with fresh device and plugin
//! instances, on the controller thread. Meanwhile the live engine would keep processing the
//! whole project on the audio thread and its workers: two heavy jobs competing for the
//! CPU. So while it runs the live engine is suspended (`EngineBridge::set_suspended`, as
//! for a project load) and the transport plays the render's master as far as it is
//! rendered instead (`EngineBridge::begin_render_preview`, `ether_core::render_preview`):
//! the timeline becomes playable as the render advances.
//!
//! - [`TransportState::rendering`](ether_core::protocol::transport::TransportState) carries
//!   the render's timeline, the rendered part (`until`) and the job's progress, at most
//!   every [`REPORT_MS`].
//! - Play needs a rendered position (`error.render.not_rendered`, or
//!   `error.render.no_preview` when the render has nothing to play), `Locate` is clamped
//!   to the rendered part (with a hint), record is refused (`error.render.running`).
//!   Playback stops where the rendered part ends (a hint says why).
//! - Edits apply to the document as usual. The render took a copy of the project when it
//!   started, so neither the files nor the preview contain them; the live engine (whose
//!   graph keeps being republished) plays them once the render ends.
//! - Done, failed, cancelled or dropped with its project, the preview is dropped and the
//!   engine resumed (unless a project load still holds it): its first block resets every
//!   node, chases notes and re-sends automation, as after a load. A preview that was
//!   playing goes on, live, from where it is. No priming: the live plugins are warm.
//! - The render isn't paced to real time, and with the live engine's cores free it renders
//!   on worker threads (`EngineBridge::offline_executor`) with a larger tick budget
//!   (`export`'s `RENDERING_TICK_BUDGET_MS`).
//! - Freeze, bounce and consolidate renders don't suspend the engine: they render one
//!   track (short, usually), keep the live session audible, and have no master to preview.

use ether_core::protocol::model::Beats;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::{RenderingState, TransportCommand};
use ether_core::protocol::{Command, CommandError, ErrorCode, NotificationLevel};

use crate::handlers::notify;
use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// How often the transport state reports the render's progress.
pub const REPORT_MS: u64 = 100;
/// Positions closer than this to the end of the rendered part count as past it (beats).
const EDGE: f64 = 1e-6;

/// The render in progress, if any.
#[derive(Debug, Default)]
pub struct RenderSuspension {
    current: Option<Rendering>,
    /// The engine clock (sample time) when play was last asked for during the render.
    play_sample: Option<u64>,
}

#[derive(Debug)]
struct Rendering {
    /// As of now.
    state: RenderingState,
    /// As last reported (`TransportState::rendering`), and when.
    shown: RenderingState,
    shown_at: u64,
}

impl RenderSuspension {
    /// A render suspends the engine.
    pub fn active(&self) -> bool {
        self.current.is_some()
    }

    /// What the transport state reports.
    pub fn state(&self) -> Option<RenderingState> {
        self.current.as_ref().map(|r| r.shown)
    }

    /// The render's timeline as of now (not throttled).
    pub fn latest(&self) -> Option<RenderingState> {
        self.current.as_ref().map(|r| r.state)
    }
}

fn keyed(key: &str) -> CommandError {
    crate::tx::keyed_err(ErrorCode::InvalidState, crate::text::msg(key))
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Suspend the engine if a load or a render needs it, resume it otherwise.
    pub fn sync_suspension(&mut self) {
        let suspended = self.loading.suspends() || self.rendering.active();
        let _ = self.bridge.set_suspended(suspended);
    }

    /// The export job just started: suspend the engine and start its preview. With the
    /// engine off (or elsewhere) there is nothing to suspend.
    pub(crate) fn begin_render(&mut self) {
        let now = self.host.now_ms();
        let Some(job) = self.export.job.as_deref_mut() else {
            return;
        };
        if self.engine.off {
            return;
        }
        let preview = match job.preview_spec() {
            Some(spec) => self.bridge.begin_render_preview(spec).is_ok(),
            None => false,
        };
        if preview {
            job.start_preview();
        }
        // The live engine's cores are free: the render may use worker threads.
        job.render_in_parallel();
        let (start, end) = job.timeline();
        let state = RenderingState {
            start,
            end,
            until: start,
            progress: 0.0,
            preview,
        };
        // Live playback stops; play again to hear the render.
        if self.transport.playing {
            let _ = self.stop();
        }
        self.rendering.current = Some(Rendering {
            state,
            shown: state,
            shown_at: now,
        });
        self.sync_suspension();
    }

    /// After the export job's tick: how far it is (reported every [`REPORT_MS`]).
    pub(crate) fn render_progress(&mut self) {
        let now = self.host.now_ms();
        let (Some(job), Some(r)) = (self.export.job.as_deref(), self.rendering.current.as_mut())
        else {
            return;
        };
        if r.state.preview {
            r.state.until = job.preview_until();
        }
        r.state.progress = job.progress();
        if now.saturating_sub(r.shown_at) >= REPORT_MS {
            r.shown = r.state;
            r.shown_at = now;
        }
    }

    /// The render ended (done, failed, cancelled, its project closed): drop the preview and
    /// resume the engine (unless a load holds it).
    pub(crate) fn end_render(&mut self) {
        let Some(r) = self.rendering.current.take() else {
            return;
        };
        self.rendering.play_sample = None;
        if r.state.preview {
            self.bridge.end_render_preview();
        }
        self.sync_suspension();
    }

    /// Play was sent to the engine: remember its clock (see [`Self::watch_preview_stop`]).
    pub(crate) fn note_play(&mut self) {
        if self.rendering.active() {
            self.rendering.play_sample = self.outputs.playhead.map(|p| p.sample_time);
        }
    }

    /// After the playhead poll: the preview may stop right after it started (played from
    /// just before the end of the rendered part), before any poll saw it play. The engine
    /// applied the play within two blocks; once its clock is well past that and it still
    /// doesn't play, it stopped by itself.
    pub(crate) fn watch_preview_stop(&mut self, out: &mut dyn MessageSink) {
        let (Some(from), Some(ph)) = (self.rendering.play_sample, self.outputs.playhead) else {
            return;
        };
        let margin = u64::from(self.config.engine_sample_rate) / 20;
        if !self.rendering.active() || !self.transport.playing || ph.playing {
            if ph.playing {
                self.rendering.play_sample = None;
            }
            return;
        }
        if ph.sample_time >= from + margin {
            self.rendering.play_sample = None;
            self.transport.playing = false;
            self.preview_stopped(out);
        }
    }

    /// Refuse `command` while a render runs: record, and play where nothing is rendered.
    pub fn refuse_while_rendering(&self, command: &Command) -> Result<(), CommandError> {
        let Some(r) = self.rendering.latest() else {
            return Ok(());
        };
        match command {
            Command::Recording(RecordingCommand::SetRecording { enabled: true }) => {
                Err(keyed("error.render.running"))
            }
            Command::Transport(TransportCommand::Play)
            | Command::Transport(TransportCommand::TogglePlay)
                if !self.transport.playing =>
            {
                if !r.preview {
                    return Err(keyed("error.render.no_preview"));
                }
                let at = self.transport.position.0;
                if at < r.start.0 - EDGE || at >= r.until.0 - EDGE {
                    return Err(keyed("error.render.not_rendered"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// While a render runs, a locate lands in the rendered part (with a hint when it had
    /// to move).
    pub(crate) fn clamp_to_rendered(&self, position: Beats, out: &mut dyn MessageSink) -> Beats {
        let Some(r) = self.rendering.latest() else {
            return position;
        };
        let clamped = Beats(position.0.clamp(r.start.0, r.until.0.max(r.start.0)));
        if (clamped.0 - position.0).abs() > EDGE {
            notify(
                out,
                NotificationLevel::Info,
                crate::text::msg("notice.render.not_rendered_yet"),
            );
        }
        clamped
    }

    /// The engine stopped by itself while the preview played: it reached the part that
    /// isn't rendered yet.
    pub(crate) fn preview_stopped(&self, out: &mut dyn MessageSink) {
        if self
            .rendering
            .latest()
            .is_some_and(|r| r.until.0 < r.end.0 - EDGE)
        {
            notify(
                out,
                NotificationLevel::Info,
                crate::text::msg("notice.render.preview_end"),
            );
        }
    }
}
