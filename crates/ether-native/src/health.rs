//! Engine health: what the controller thread and the main thread are doing, and a watchdog
//! that reports when the controller thread stays stuck on one step.
//!
//! The controller thread runs commands and the 60 Hz tick, and waits for main-thread
//! plugin work ([`crate::plugins::PluginHost`]) along the way. When it stalls, the playhead
//! stops and commands queue, while audio (its own thread) keeps playing. To say why, each
//! step records itself in [`Health`] with a couple of relaxed atomic stores (no locks or
//! allocations on the hot path), and the [`Watchdog`] thread reads them 4 times a second:
//! a step running longer than [`STALL`] is logged and reported to the UI as
//! `EngineEvent::Unresponsive`, then `Responsive` when it ends.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ether_core::protocol::ServerMessage;
use ether_core::protocol::engine::EngineEvent;
use ether_core::protocol::message::Event;
use ether_core::protocol::model::DeviceId;

thread_local! {
    /// Set on the controller thread: only its waits for the main thread are recorded (other
    /// threads' calls would overwrite each other's markers).
    static CONTROLLER_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Told every event the watchdog reports ([`set_watchdog_observer`]).
pub type WatchdogObserver = fn(&EngineEvent);

static WATCHDOG_OBSERVER: std::sync::OnceLock<WatchdogObserver> = std::sync::OnceLock::new();

/// Extension point: `observer` is also told every stall event the [`Watchdog`] reports (e.g.
/// to count stalls), from the watchdog thread. Set once, at start.
pub fn set_watchdog_observer(observer: WatchdogObserver) {
    let _ = WATCHDOG_OBSERVER.set(observer);
}

/// Mark the current thread as the controller thread (once, when it starts).
pub fn set_controller_thread() {
    CONTROLLER_THREAD.with(|c| c.set(true));
}

/// A controller step running longer than this is a stall.
pub const STALL: Duration = Duration::from_secs(2);
/// How often the watchdog looks.
const CHECK_EVERY: Duration = Duration::from_millis(250);

/// What a thread is busy with (the controller thread's step, or a main-thread plugin call).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Work {
    Idle = 0,
    Command,
    Tick,
    PollPlugins,
    LoadPlugin,
    UnloadPlugin,
    OpenEditor,
    CloseEditor,
    SaveState,
    ReadParams,
    PluginCall,
}

impl Work {
    fn from_u8(v: u8) -> Self {
        use Work::*;
        [
            Idle,
            Command,
            Tick,
            PollPlugins,
            LoadPlugin,
            UnloadPlugin,
            OpenEditor,
            CloseEditor,
            SaveState,
            ReadParams,
            PluginCall,
        ]
        .get(usize::from(v))
        .copied()
        .unwrap_or(PluginCall)
    }

    /// The main-thread call, as a whole.
    fn call(self) -> &'static str {
        match self {
            Work::PollPlugins => "the regular plugin update",
            Work::LoadPlugin => "loading a plugin",
            Work::UnloadPlugin => "unloading a plugin",
            Work::OpenEditor => "opening a plugin editor",
            Work::CloseEditor => "closing a plugin editor",
            Work::SaveState => "saving a plugin's state",
            Work::ReadParams => "reading a plugin's parameters",
            Work::Idle | Work::Command | Work::Tick | Work::PluginCall => "a plugin call",
        }
    }

    /// Short name for logs.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Work::Idle => "idle",
            Work::Command => "command",
            Work::Tick => "tick",
            Work::PollPlugins => "poll",
            Work::LoadPlugin => "load",
            Work::UnloadPlugin => "unload",
            Work::OpenEditor => "open-editor",
            Work::CloseEditor => "close-editor",
            Work::SaveState => "save-state",
            Work::ReadParams => "read-params",
            Work::PluginCall => "call",
        }
    }

    /// What a plugin is doing during this main-thread call.
    fn doing(self) -> &'static str {
        match self {
            Work::PollPlugins => "handling its editor and timers",
            Work::LoadPlugin => "loading",
            Work::UnloadPlugin => "unloading",
            Work::OpenEditor => "opening its editor",
            Work::CloseEditor => "closing its editor",
            Work::SaveState => "saving its state",
            Work::ReadParams => "reading its parameters",
            Work::Idle | Work::Command | Work::Tick | Work::PluginCall => "running a call",
        }
    }
}

/// One thread's current [`Work`] and since when (ms since [`Health`]'s epoch, +1; 0 = idle).
#[derive(Debug, Default)]
struct Slot {
    since: AtomicU64,
    work: AtomicU8,
}

impl Slot {
    fn get(&self) -> Option<(Work, u64)> {
        match self.since.load(Ordering::Relaxed) {
            0 => None,
            since => Some((Work::from_u8(self.work.load(Ordering::Relaxed)), since - 1)),
        }
    }

    fn set(&self, state: Option<(Work, u64)>) {
        match state {
            Some((work, at)) => {
                self.work.store(work as u8, Ordering::Relaxed);
                self.since.store(at + 1, Ordering::Relaxed);
            }
            None => self.since.store(0, Ordering::Relaxed),
        }
    }
}

/// Shared activity markers (see the module docs). Cheap to update from any thread.
#[derive(Debug)]
pub struct Health {
    epoch: Instant,
    /// The controller thread's current step.
    controller: Slot,
    /// The main-thread call the controller thread is waiting for (queued or running).
    waiting: Slot,
    /// The plugin call running on the main thread, and the device it is for.
    main: Slot,
    main_device: Mutex<Option<DeviceId>>,
    /// Display names of plugin devices, for reasons ("Analog Lab V is busy …").
    names: Mutex<HashMap<DeviceId, String>>,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            controller: Slot::default(),
            waiting: Slot::default(),
            main: Slot::default(),
            main_device: Mutex::default(),
            names: Mutex::default(),
        }
    }
}

/// Restores a slot's previous state when dropped (steps can nest: a plugin's nested run
/// loop may run another main-thread call inside one).
#[must_use]
pub struct Mark<'a> {
    slot: &'a Slot,
    previous: Option<(Work, u64)>,
}

impl Drop for Mark<'_> {
    fn drop(&mut self) {
        self.slot.set(self.previous);
    }
}

impl Health {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn mark<'a>(&self, slot: &'a Slot, work: Work) -> Mark<'a> {
        let previous = slot.get();
        slot.set(Some((work, self.now_ms())));
        Mark { slot, previous }
    }

    /// The controller thread starts a step (ends when the mark drops).
    pub fn controller_step(&self, work: Work) -> Mark<'_> {
        self.mark(&self.controller, work)
    }

    /// The current thread waits for a main-thread call (recorded on the controller thread
    /// only; `None` elsewhere).
    pub fn waiting_for_main(&self, work: Work) -> Option<Mark<'_>> {
        CONTROLLER_THREAD
            .with(Cell::get)
            .then(|| self.mark(&self.waiting, work))
    }

    /// A plugin call starts running on the main thread, for `device` when known.
    pub fn main_call(&self, work: Work, device: Option<DeviceId>) -> Mark<'_> {
        self.set_main_device(device);
        self.mark(&self.main, work)
    }

    /// The device the running main-thread call is working on now (a poll goes through
    /// every plugin).
    pub fn set_main_device(&self, device: Option<DeviceId>) {
        if let Ok(mut d) = self.main_device.lock() {
            *d = device;
        }
    }

    /// Remember a plugin device's display name.
    pub fn name_device(&self, device: DeviceId, name: &str) {
        if let Ok(mut names) = self.names.lock()
            && names.get(&device).map(String::as_str) != Some(name)
        {
            names.insert(device, name.to_string());
        }
    }

    fn device_name(&self, device: Option<DeviceId>) -> Option<String> {
        let names = self.names.lock().ok()?;
        names.get(&device?).cloned()
    }

    /// The display name of a plugin device (its id when unnamed), for logs.
    pub(crate) fn device_label(&self, device: Option<DeviceId>) -> Option<String> {
        let device = device?;
        Some(
            self.device_name(Some(device))
                .unwrap_or_else(|| device.to_string()),
        )
    }

    /// The current stall, if the controller thread has been on one step for [`STALL`].
    pub fn stall(&self) -> Option<Stall> {
        let now = self.now_ms();
        let (step, since) = self.controller.get()?;
        let stalled_ms = now.saturating_sub(since);
        (stalled_ms >= STALL.as_millis() as u64).then(|| Stall {
            stalled_ms,
            reason: self.reason(step),
        })
    }

    fn reason(&self, step: Work) -> Option<String> {
        if let Some((waited, _)) = self.waiting.get() {
            return Some(match self.main.get() {
                Some((running, _)) => {
                    let device = self.main_device.lock().ok().and_then(|d| *d);
                    let who = self
                        .device_name(device)
                        .unwrap_or_else(|| "a plugin".into());
                    format!("{who} is busy ({})", running.doing())
                }
                None => format!(
                    "waiting for the main thread ({}), which is busy outside the engine: a \
                     dialog, a window being moved or resized, or a plugin window",
                    waited.call()
                ),
            });
        }
        match step {
            Work::Command => Some("processing a command".into()),
            Work::Tick => Some("running its update loop".into()),
            _ => None,
        }
    }
}

/// A stall: how long the controller thread has been stuck, and why if known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stall {
    pub stalled_ms: u64,
    pub reason: Option<String>,
}

/// The watchdog thread (see the module docs). Stops when dropped.
#[derive(Debug)]
pub struct Watchdog {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Watch `health`, logging stalls and sending their events through `emit` (from the
    /// watchdog thread: the controller thread is the one that is stuck).
    pub fn start(
        health: Arc<Health>,
        emit: impl Fn(ServerMessage) + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("ether-watchdog".into())
                .spawn(move || {
                    let mut tracker = Tracker::default();
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(CHECK_EVERY);
                        for event in tracker.observe(health.stall(), Instant::now()) {
                            if let Some(observe) = WATCHDOG_OBSERVER.get() {
                                observe(&event);
                            }
                            emit(ServerMessage::Event(Event::Engine { event }));
                        }
                    }
                })?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Turns successive observations into `Unresponsive` / `Responsive` events (and logs).
#[derive(Debug, Default)]
struct Tracker {
    /// The stall being reported: when it started, and the last reason sent.
    current: Option<(Instant, Option<String>)>,
}

impl Tracker {
    fn observe(&mut self, stall: Option<Stall>, now: Instant) -> Vec<EngineEvent> {
        let ms = |d: Duration| d.as_millis().min(u128::from(u32::MAX)) as u32;
        match (stall, &mut self.current) {
            (Some(s), None) => {
                let started = now - Duration::from_millis(s.stalled_ms);
                tracing::warn!(
                    stalled_ms = s.stalled_ms,
                    reason = s.reason.as_deref().unwrap_or("unknown"),
                    "engine unresponsive"
                );
                self.current = Some((started, s.reason.clone()));
                vec![EngineEvent::Unresponsive {
                    stalled_ms: ms(now - started),
                    reason: s.reason,
                }]
            }
            (Some(s), Some((started, reason))) if s.reason != *reason && s.reason.is_some() => {
                tracing::warn!(
                    stalled_ms = s.stalled_ms,
                    reason = s.reason.as_deref().unwrap_or("unknown"),
                    "engine still unresponsive"
                );
                *reason = s.reason.clone();
                vec![EngineEvent::Unresponsive {
                    stalled_ms: ms(now - *started),
                    reason: s.reason,
                }]
            }
            (Some(_), Some(_)) => Vec::new(),
            (None, Some(_)) => {
                let (started, reason) = self.current.take().expect("matched Some");
                let stalled_ms = ms(now - started);
                tracing::warn!(
                    stalled_ms,
                    reason = reason.as_deref().unwrap_or("unknown"),
                    "engine responsive again"
                );
                vec![EngineEvent::Responsive { stalled_ms, reason }]
            }
            (None, None) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health_at(ago: Duration) -> Health {
        Health {
            epoch: Instant::now() - ago - Duration::from_secs(1),
            ..Health::default()
        }
    }

    /// Back-date a slot: as if `work` started `ago` before now.
    fn started(h: &Health, slot: &Slot, work: Work, ago: Duration) {
        let at = h.now_ms().saturating_sub(ago.as_millis() as u64);
        slot.set(Some((work, at)));
    }

    /// The watchdog's events reach the UI and the observer (`set_watchdog_observer`).
    #[test]
    fn the_watchdog_tells_its_observer_too() {
        static SEEN: Mutex<Vec<EngineEvent>> = Mutex::new(Vec::new());
        set_watchdog_observer(|e| SEEN.lock().unwrap().push(e.clone()));
        let h = Arc::new(health_at(Duration::from_secs(10)));
        started(&h, &h.controller, Work::Tick, Duration::from_secs(3));
        let (tx, rx) = std::sync::mpsc::channel();
        let dog = Watchdog::start(h.clone(), move |m| {
            let _ = tx.send(m);
        })
        .unwrap();
        let sent = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a stall event");
        drop(dog);
        let ServerMessage::Event(Event::Engine { event }) = sent else {
            panic!("{sent:?}")
        };
        assert!(
            matches!(event, EngineEvent::Unresponsive { .. }),
            "{event:?}"
        );
        assert_eq!(SEEN.lock().unwrap().first(), Some(&event));
    }

    #[test]
    fn short_steps_are_not_stalls_and_marks_restore() {
        let h = Health::default();
        {
            let _tick = h.controller_step(Work::Tick);
            assert!(h.stall().is_none());
            {
                let _inner = h.controller_step(Work::Command);
                assert_eq!(h.controller.get().map(|s| s.0), Some(Work::Command));
            }
            assert_eq!(h.controller.get().map(|s| s.0), Some(Work::Tick));
        }
        assert!(h.controller.get().is_none());
    }

    #[test]
    fn reasons_name_the_busy_plugin_or_the_blocked_main_thread() {
        let h = health_at(Duration::from_secs(10));
        let d = DeviceId::NIL;
        h.name_device(d, "Analog Lab V");
        started(&h, &h.controller, Work::Tick, Duration::from_secs(3));

        // The tick itself is slow.
        let s = h.stall().expect("stalled");
        assert!(s.stalled_ms >= 3000);
        assert_eq!(s.reason.as_deref(), Some("running its update loop"));

        // Other threads' waits are not recorded.
        assert!(h.waiting_for_main(Work::PollPlugins).is_none());

        // Waiting for a main thread that hasn't picked the call up.
        started(&h, &h.waiting, Work::PollPlugins, Duration::from_secs(3));
        let r = h.stall().unwrap().reason.unwrap();
        assert!(
            r.starts_with("waiting for the main thread (the regular plugin update), which is busy"),
            "{r}"
        );

        // The main thread is running the call, in a plugin.
        let _main = h.main_call(Work::OpenEditor, Some(d));
        assert_eq!(
            h.stall().unwrap().reason.as_deref(),
            Some("Analog Lab V is busy (opening its editor)")
        );
    }

    #[test]
    fn tracker_reports_start_reason_changes_and_end_once() {
        let mut t = Tracker::default();
        let now = Instant::now();
        let stall = |ms, reason: &str| {
            Some(Stall {
                stalled_ms: ms,
                reason: Some(reason.into()),
            })
        };
        assert!(t.observe(None, now).is_empty());
        let e = t.observe(stall(2100, "a"), now);
        assert!(
            matches!(&e[..], [EngineEvent::Unresponsive { stalled_ms: 2100, reason: Some(r) }] if r == "a")
        );
        assert!(
            t.observe(stall(2350, "a"), now + Duration::from_millis(250))
                .is_empty()
        );
        let e = t.observe(stall(2600, "b"), now + Duration::from_millis(500));
        assert!(
            matches!(&e[..], [EngineEvent::Unresponsive { stalled_ms: 2600, reason: Some(r) }] if r == "b")
        );
        let e = t.observe(None, now + Duration::from_millis(900));
        assert!(
            matches!(&e[..], [EngineEvent::Responsive { stalled_ms: 3000, reason: Some(r) }] if r == "b")
        );
        assert!(
            t.observe(None, now + Duration::from_millis(1200))
                .is_empty()
        );
    }
}
