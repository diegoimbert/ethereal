//! A project load: the engine's audio is suspended until the project is ready.
//!
//! Opening a project (or a version, a recovery, a joined session: everything that goes
//! through [`EtherController::load_project`]), and loading the plugins held back by safe
//! mode, instantiate plugins progressively over several ticks
//! ([`crate::engine::PLUGIN_LOAD_BUDGET_MS`]) and decode media over several more. Meanwhile
//! the audio thread would run every partial graph while plugins and the controller load
//! around it, fall behind and play glitchy catch-up for a while. Instead the load suspends
//! the engine (`EngineBridge::set_suspended`: the callback outputs silence and runs no
//! graph work; nodes, graphs and media still arrive) and resumes it once the last plugin
//! is in, the media are loaded and the final graph is published. The engine starts that
//! block from a clean state (`ether_core::Engine`'s resume).
//!
//! With plugins, the load then lasts [`PRIME_MS`] more of rendered audio (the engine's
//! clock, as the playhead reports it): the plugins' first blocks (their lazy setup, often
//! slow) run with the transport stopped, so whatever they cost is over before anything can
//! play.
//!
//! While loading, [`TransportState::loading`] carries the progress, and play, stop and
//! record are refused (`error.project.loading`), whoever asks (the UI, its shortcuts,
//! MIDI-mapped controls, the agent tools, the API). Locate works (the playhead moves).
//! Edits work as usual: they apply to the document at once and are heard when the load
//! ends (the graph keeps being republished while suspended).

use ether_core::protocol::message::Command;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::{LoadProgress, TransportCommand};
use ether_core::protocol::{CommandError, ErrorCode};

use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Audio the engine renders after the resume before the load ends (the plugins' first
/// blocks).
pub const PRIME_MS: u64 = 150;
/// The load ends anyway this long after the resume when the engine's clock doesn't
/// advance (no audio device running).
pub const PRIME_TIMEOUT_MS: u64 = 2_000;

/// The load in progress, if any.
#[derive(Debug, Default)]
pub struct LoadState {
    current: Option<Load>,
    /// The bridge reports the engine's clock (the playhead): real engines do from their
    /// first tick, test bridges may not (their loads end at the resume, unprimed).
    clock_seen: bool,
}

#[derive(Debug)]
struct Load {
    /// The most media load jobs seen (their total: jobs only ever finish during a load).
    media_total: u32,
    progress: LoadProgress,
    /// Resumed, priming: since when (wall ms), and the engine sample time it started at
    /// (the first one seen since).
    resumed: Option<(u64, Option<u64>)>,
}

impl LoadState {
    /// A load is in progress.
    pub fn active(&self) -> bool {
        self.current.is_some()
    }

    /// The load holds the engine suspended (it hasn't resumed it yet).
    pub fn suspends(&self) -> bool {
        self.current.as_ref().is_some_and(|l| l.resumed.is_none())
    }

    /// The progress of the load in progress.
    pub fn progress(&self) -> Option<LoadProgress> {
        self.current.as_ref().map(|l| l.progress.clone())
    }
}

/// Commands refused while a project loads: they start or stop the transport.
pub fn starts_or_stops_transport(command: &Command) -> bool {
    matches!(
        command,
        Command::Transport(
            TransportCommand::Play | TransportCommand::Stop | TransportCommand::TogglePlay
        ) | Command::Recording(RecordingCommand::SetRecording { .. })
    )
}

pub fn loading_error() -> CommandError {
    crate::tx::keyed_err(
        ErrorCode::InvalidState,
        crate::text::msg("error.project.loading"),
    )
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// A load starts: suspend the engine (with the engine off there is nothing to load
    /// for). The transport is stopped by the caller (a project switch) or here (plugins
    /// loaded out of safe mode while playing).
    pub fn begin_load(&mut self) {
        if self.engine.off {
            return;
        }
        if self.transport.playing {
            let _ = self.stop();
        }
        self.loading.current = Some(Load {
            media_total: 0,
            progress: LoadProgress::default(),
            resumed: None,
        });
        self.sync_suspension();
    }

    /// Refuse `command` while a project loads (play, stop, record).
    pub fn refuse_while_loading(&self, command: &Command) -> Result<(), CommandError> {
        if self.loading.active() && starts_or_stops_transport(command) {
            return Err(loading_error());
        }
        Ok(())
    }

    /// After a publish (every tick, and the load's own): update the progress, resume the
    /// engine once everything is in, and end the load once it primed. The transport event
    /// that follows carries it.
    pub fn load_tick(&mut self, out: &mut dyn MessageSink) {
        let clock = self.outputs.playhead.map(|p| p.sample_time);
        self.loading.clock_seen |= clock.is_some();
        let Some(load) = self.loading.current.as_mut() else {
            return;
        };
        if let Some((since, start)) = load.resumed.as_mut() {
            let now = self.host.now_ms();
            let rate = u64::from(self.config.engine_sample_rate);
            let primed = match (*start, clock) {
                (Some(from), Some(t)) => t >= from + rate * PRIME_MS / 1000,
                (None, Some(t)) => {
                    *start = Some(t);
                    false
                }
                (_, None) => false,
            };
            if primed || now.saturating_sub(*since) >= PRIME_TIMEOUT_MS {
                self.loading.current = None;
            }
            self.emit_transport_if_changed(out);
            return;
        }
        let pending_media = self.media.load_jobs() as u32;
        load.media_total = load.media_total.max(pending_media);
        let (plugins_loaded, plugins_total) = match self.doc.as_ref() {
            Some(doc) => self.engine.plugin_progress(&doc.project),
            None => (0, 0),
        };
        load.progress = LoadProgress {
            plugins_loaded,
            plugins_total,
            media_loaded: load.media_total - pending_media,
            media_total: load.media_total,
            media_pending: if pending_media == 0 {
                Vec::new()
            } else {
                self.media.load_job_media()
            },
        };
        // Ready: no plugin waits, the last graph was accepted, no media left to load (or
        // the project was closed, or the engine turned off).
        let ready = self.doc.is_none()
            || self.engine.off
            || (!self.engine.plugins_pending() && !self.engine.graph_dirty && pending_media == 0);
        if ready {
            let clock_seen = self.loading.clock_seen;
            let load = self.loading.current.as_mut().expect("checked");
            // Only plugins prime (built-in devices have no costly first block).
            if clock_seen && plugins_total > 0 && !self.engine.off && self.doc.is_some() {
                load.resumed = Some((self.host.now_ms(), None));
            } else {
                self.loading.current = None;
            }
            // Resumed, unless a render holds it (`rendering`).
            self.sync_suspension();
        }
        self.emit_transport_if_changed(out);
    }
}
