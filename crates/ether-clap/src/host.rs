//! The clack `HostHandlers` implementation: what a plugin instance can call back into.
//!
//! Callbacks only record what happened (atomics on the shared half, `Cell`s on the main-thread
//! half). [`crate::ClapPlugin::poll`] turns them into `PluginNotification`s on the main thread.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use clack_extensions::audio_ports::{
    AudioPortRescanFlags, HostAudioPorts, HostAudioPortsImpl, PluginAudioPorts,
};
use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl, PluginGui};
use clack_extensions::latency::{HostLatency, HostLatencyImpl, PluginLatency};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::note_ports::{
    HostNotePorts, HostNotePortsImpl, NoteDialects, NotePortRescanFlags, PluginNotePorts,
};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
    PluginParams,
};
use clack_extensions::state::{HostState, HostStateImpl, PluginState};
use clack_extensions::timer::{HostTimer, HostTimerImpl, PluginTimer, TimerId};
use clack_host::prelude::*;

pub(crate) struct EtherHost;

impl HostHandlers for EtherHost {
    type Shared<'a> = HostShared;
    type MainThread<'a> = HostMainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostLog>()
            .register::<HostParams>()
            .register::<HostState>()
            .register::<HostLatency>()
            .register::<HostGui>()
            .register::<HostTimer>()
            .register::<HostAudioPorts>()
            .register::<HostNotePorts>();
    }
}

/// Thread-safe half: flags set from any plugin thread.
#[derive(Default)]
pub(crate) struct HostShared {
    pub callback_requested: AtomicBool,
    pub restart_requested: AtomicBool,
    pub flush_requested: AtomicBool,
    pub gui_closed: AtomicBool,
    /// Pending editor resize request (`GuiSize::pack_to_u64`), 0 = none.
    pub gui_resize: AtomicU64,
}

impl HostShared {
    pub fn take(flag: &AtomicBool) -> bool {
        flag.swap(false, Ordering::AcqRel)
    }
}

impl<'a> SharedHandler<'a> for HostShared {
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::Release);
    }

    fn request_process(&self) {
        // The engine always processes active nodes; nothing to do.
    }

    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::Release);
    }
}

impl HostLogImpl for HostShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        // Plugins may log from the audio thread; hosts decide whether a subscriber is RT-safe.
        match severity {
            LogSeverity::Debug => {}
            LogSeverity::Info => tracing::info!(target: "clap_plugin", "{message}"),
            LogSeverity::Warning => tracing::warn!(target: "clap_plugin", "{message}"),
            _ => tracing::error!(target: "clap_plugin", "{severity}: {message}"),
        }
    }
}

impl HostParamsImplShared for HostShared {
    fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::Release);
    }
}

impl HostGuiImpl for HostShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        // Applied to the host window (if any) on the next poll.
        self.gui_resize
            .store(new_size.pack_to_u64().max(1), Ordering::Release);
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.gui_closed.store(true, Ordering::Release);
    }
}

/// Plugin extensions, queried once after init.
#[derive(Clone, Copy, Default)]
pub(crate) struct PluginExts {
    pub params: Option<PluginParams>,
    pub state: Option<PluginState>,
    pub latency: Option<PluginLatency>,
    pub gui: Option<PluginGui>,
    pub timer: Option<PluginTimer>,
    pub audio_ports: Option<PluginAudioPorts>,
    pub note_ports: Option<PluginNotePorts>,
}

pub(crate) struct Timer {
    pub id: TimerId,
    pub period: Duration,
    pub next: Instant,
}

/// Main-thread half: flags + timers, only touched on the plugin's main thread.
pub(crate) struct HostMainThread<'a> {
    _shared: &'a HostShared,
    pub exts: Cell<PluginExts>,
    pub latency_changed: Cell<bool>,
    pub params_rescan: Cell<bool>,
    pub state_dirty: Cell<bool>,
    pub timers: RefCell<Vec<Timer>>,
    next_timer_id: Cell<u32>,
}

impl<'a> HostMainThread<'a> {
    pub fn new(shared: &'a HostShared) -> Self {
        Self {
            _shared: shared,
            exts: Cell::new(PluginExts::default()),
            latency_changed: Cell::new(false),
            params_rescan: Cell::new(false),
            state_dirty: Cell::new(false),
            timers: RefCell::new(Vec::new()),
            next_timer_id: Cell::new(0),
        }
    }

    /// Timers that are due now (and re-arms them).
    pub fn due_timers(&self, now: Instant) -> Vec<TimerId> {
        let mut due = Vec::new();
        for t in self.timers.borrow_mut().iter_mut() {
            if now >= t.next {
                due.push(t.id);
                t.next = now + t.period;
            }
        }
        due
    }
}

impl<'a> MainThreadHandler<'a> for HostMainThread<'a> {
    fn initialized(&self, instance: InitializedPluginHandle<'a>) {
        self.exts.set(PluginExts {
            params: instance.get_extension(),
            state: instance.get_extension(),
            latency: instance.get_extension(),
            gui: instance.get_extension(),
            timer: instance.get_extension(),
            audio_ports: instance.get_extension(),
            note_ports: instance.get_extension(),
        });
    }
}

impl HostParamsImplMainThread for HostMainThread<'_> {
    fn rescan(&self, _flags: ParamRescanFlags) {
        self.params_rescan.set(true);
    }

    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for HostMainThread<'_> {
    fn mark_dirty(&self) {
        self.state_dirty.set(true);
    }
}

impl HostLatencyImpl for HostMainThread<'_> {
    fn changed(&self) {
        self.latency_changed.set(true);
    }
}

impl HostTimerImpl for HostMainThread<'_> {
    fn register_timer(&self, period_ms: u32) -> Result<TimerId, HostError> {
        // CLAP recommends hosts clamp very short periods; poll() runs at ~30-60 Hz anyway.
        let period = Duration::from_millis(u64::from(period_ms.max(10)));
        let id = TimerId(self.next_timer_id.get());
        self.next_timer_id.set(id.0.wrapping_add(1));
        self.timers.borrow_mut().push(Timer {
            id,
            period,
            next: Instant::now() + period,
        });
        Ok(id)
    }

    fn unregister_timer(&self, timer_id: TimerId) -> Result<(), HostError> {
        let mut timers = self.timers.borrow_mut();
        let before = timers.len();
        timers.retain(|t| t.id != timer_id);
        if timers.len() == before {
            Err(HostError::Message("unknown timer id"))
        } else {
            Ok(())
        }
    }
}

impl HostAudioPortsImpl for HostMainThread<'_> {
    fn is_rescan_flag_supported(&self, _flag: AudioPortRescanFlags) -> bool {
        false
    }

    fn rescan(&self, _flags: AudioPortRescanFlags) {
        // Port changes need a restart; the plugin is expected to call request_restart.
    }
}

impl HostNotePortsImpl for HostMainThread<'_> {
    fn supported_dialects(&self) -> NoteDialects {
        NoteDialects::CLAP | NoteDialects::MIDI
    }

    fn rescan(&self, _flags: NotePortRescanFlags) {}
}

pub(crate) fn host_info() -> HostInfo {
    HostInfo::new(
        "Ethereal",
        "Ethereal contributors",
        "https://github.com/diegoimbert/ethereal",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("static host info has no NUL bytes")
}
