//! Losing the audio output device mid-session.
//!
//! An unplugged USB interface or Bluetooth headphones switched off kill the cpal stream for
//! good: CoreAudio and WASAPI report `DeviceNotAvailable` ([`crate::audio::on_stream_error`]),
//! ALSA a stream of backend errors while callbacks stop (detected here as a stall). A panic
//! in the render callback silences the stream the same way. Each controller tick checks
//! for it ([`AudioState::check_device`]) and, instead of a silently dead engine:
//! 1. reports `running: false` and a warning notification naming the device;
//! 2. restarts the engine on the configured output (`default` follows the OS's new
//!    default), else the default output, else the null backend (no sound, the engine and
//!    the UI keep working);
//! 3. while it runs on a fallback, looks for the chosen device every [`DEVICE_PROBE`] and
//!    switches back when it is plugged in again (an info notification).
//!
//! The saved settings never change: the fallback is temporary. Choosing a device in the
//! audio settings ends it.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ether_core::Engine;
use ether_core::protocol::ServerMessage;
use ether_core::protocol::message::NotificationLevel;

use super::{AudioState, notification, status_event};
use crate::audio::{self, AudioBackendKind, AudioOutput, AudioSettings};
use ether_controller::text::{Msg, msg};

/// How often the chosen device is looked for while a fallback runs.
pub(super) const DEVICE_PROBE: Duration = Duration::from_secs(2);
/// The longest wait between failed attempts to switch back to a device that is listed but
/// won't open (each failure doubles the wait).
const RETRY_MAX: Duration = Duration::from_secs(60);
/// No callback for this long while the stream reports errors: the device is gone.
const STALL: Duration = Duration::from_secs(2);
/// A second render crash this soon after a restart stops audio instead of restarting it
/// again (a crash loop would otherwise notify every tick).
const CRASH_LOOP: Duration = Duration::from_secs(10);

/// The engine runs on a fallback because the chosen device is gone.
#[derive(Debug)]
pub(super) struct Lost {
    next_probe: Instant,
    backoff: Duration,
}

impl Lost {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            next_probe: now + DEVICE_PROBE,
            backoff: DEVICE_PROBE,
        }
    }
}

/// Callback progress, for the stall check.
#[derive(Debug)]
pub(super) struct StallWatch {
    frames: u64,
    xruns: u32,
    since: Instant,
}

impl StallWatch {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            frames: 0,
            xruns: 0,
            since: now,
        }
    }
}

/// "audio output "name"", or "the audio output" when its name is unknown.
fn device_label(name: Option<&str>) -> Msg {
    match name {
        Some(n) => msg("notice.audio.output_named").arg("name", n).into(),
        None => msg("notice.audio.output_unnamed").into(),
    }
}

impl AudioState {
    /// Controller tick: handle a lost device or crashed render, or switch back to the
    /// chosen device once it is available again. Returns the messages for the UI.
    pub(super) fn check_device(&mut self, now: Instant) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        if self.released {
            // The null backend runs: no device to lose.
            return out;
        }
        let lost = self.shared.device_lost.swap(false, Ordering::SeqCst) | self.stalled(now);
        let crashed = self.shared.render_crashed.swap(false, Ordering::SeqCst);
        if lost || crashed {
            self.shared.running.store(false, Ordering::Relaxed);
            out.push(status_event(self.status()));
            let (level, message) = if lost {
                self.recover_lost(now)
            } else {
                self.recover_crash(now)
            };
            tracing::warn!(message = %message.text, "audio output");
            out.push(notification(level, message));
            out.push(status_event(self.status()));
        } else if self.lost.as_ref().is_some_and(|l| now >= l.next_probe)
            && let Some(message) = self.probe_back(now)
        {
            tracing::info!(message = %message.text, "audio output");
            out.push(notification(NotificationLevel::Info, message));
            out.push(status_event(self.status()));
        }
        out
    }

    /// The cpal stream stopped calling back while reporting errors (ALSA's unplugged
    /// device): treated like `DeviceNotAvailable`. A stall without errors (a plugin hung
    /// in the callback) is the watchdog's business, and restarting would block on it.
    fn stalled(&mut self, now: Instant) -> bool {
        let frames = self.shared.frames.load(Ordering::Relaxed);
        let xruns = self.shared.xruns.load(Ordering::Relaxed);
        let cpal = self
            .output
            .as_ref()
            .is_some_and(|o| o.info.backend == AudioBackendKind::Cpal);
        let w = &mut self.stall;
        if !cpal || frames != w.frames {
            *w = StallWatch {
                frames,
                xruns,
                since: now,
            };
            return false;
        }
        if xruns != w.xruns && now.duration_since(w.since) >= STALL {
            *w = StallWatch {
                frames,
                xruns,
                since: now,
            };
            return true;
        }
        false
    }

    /// Stop what runs and take the engine.
    fn take_engine(&mut self) -> Option<Box<Engine>> {
        match self.output.take() {
            Some(out) => out.stop(),
            None => self.parked.take(),
        }
    }

    fn started(&mut self, out: AudioOutput, now: Instant) {
        self.output = Some(out);
        self.stall = StallWatch::new(now);
    }

    /// The device is gone: restart on the configured output, else the default one, else
    /// without sound. Returns the notification.
    fn recover_lost(&mut self, now: Instant) -> (NotificationLevel, Msg) {
        let gone = self.output.as_ref().and_then(|o| o.info.device.clone());
        let what = |id: &str| msg(id).arg("device", device_label(gone.as_deref()));
        let Some(engine) = self.take_engine() else {
            return (
                NotificationLevel::Error,
                what("notice.audio.lost_engine_gone").into(),
            );
        };
        let want = self.run_settings(&self.settings);
        // As configured: `default` follows the OS to its new default device, and a device
        // that only blinked (driver reset) is back already.
        let engine = match AudioOutput::start(engine, &want, self.shared.clone()) {
            Ok(out) => {
                let now_on = out.info.device.clone();
                self.started(out, now);
                self.lost = None;
                return (
                    NotificationLevel::Warning,
                    what("notice.audio.lost_restarted")
                        .arg("now", device_label(now_on.as_deref()))
                        .into(),
                );
            }
            Err((_, engine)) => engine,
        };
        self.lost = Some(Lost::new(now));
        match self.start_fallback(engine, &want, now) {
            Ok(Some(device)) => (
                NotificationLevel::Warning,
                what("notice.audio.lost_fallback")
                    .arg("now", device_label(device.as_deref()))
                    .into(),
            ),
            Ok(None) => (
                NotificationLevel::Warning,
                what("notice.audio.lost_silent").into(),
            ),
            Err(e) => (
                NotificationLevel::Error,
                what("notice.audio.lost_failed").arg("error", e).into(),
            ),
        }
    }

    /// The render panicked (the stream outputs silence): restart it once; a crash loop
    /// stops audio until the user picks a device again.
    fn recover_crash(&mut self, now: Instant) -> (NotificationLevel, Msg) {
        let again = self
            .last_crash
            .is_some_and(|t| now.duration_since(t) < CRASH_LOOP);
        self.last_crash = Some(now);
        let Some(engine) = self.take_engine() else {
            return (
                NotificationLevel::Error,
                msg("notice.audio.crash_lost").into(),
            );
        };
        if again {
            self.parked = Some(engine);
            return (
                NotificationLevel::Error,
                msg("notice.audio.crash_loop").into(),
            );
        }
        let want = self.run_settings(&self.settings);
        match AudioOutput::start(engine, &want, self.shared.clone()) {
            Ok(out) => {
                self.started(out, now);
                (
                    NotificationLevel::Error,
                    msg("notice.audio.crash_restarted").into(),
                )
            }
            Err((e, engine)) => {
                self.parked = Some(engine);
                (
                    NotificationLevel::Error,
                    msg("notice.audio.crash_failed")
                        .arg("error", e.to_string())
                        .into(),
                )
            }
        }
    }

    /// Run `engine` on the default output (when `want` names another device), else on the
    /// null backend. `Ok(Some(device))`: on the default output; `Ok(None)`: without sound.
    fn start_fallback(
        &mut self,
        engine: Box<Engine>,
        want: &AudioSettings,
        now: Instant,
    ) -> Result<Option<Option<String>>, String> {
        let mut engine = engine;
        if want.backend == AudioBackendKind::Cpal && want.output_device.is_some() {
            let default = AudioSettings {
                output_device: None,
                ..want.clone()
            };
            match AudioOutput::start(engine, &default, self.shared.clone()) {
                Ok(out) => {
                    let device = out.info.device.clone();
                    self.started(out, now);
                    return Ok(Some(device));
                }
                Err((_, e)) => engine = e,
            }
        }
        let null = AudioSettings {
            backend: AudioBackendKind::Null,
            ..want.clone()
        };
        match AudioOutput::start(engine, &null, self.shared.clone()) {
            Ok(out) => {
                self.started(out, now);
                Ok(None)
            }
            Err((e, engine)) => {
                self.parked = Some(engine);
                Err(e.to_string())
            }
        }
    }

    /// A fallback runs: switch back to the chosen device if it is there again.
    fn probe_back(&mut self, now: Instant) -> Option<Msg> {
        let want = self.run_settings(&self.settings);
        if !audio::output_available(&want) {
            if let Some(l) = &mut self.lost {
                l.next_probe = now + DEVICE_PROBE;
            }
            return None;
        }
        let Some(engine) = self.take_engine() else {
            self.lost = None;
            return Some(msg("notice.audio.engine_lost").into());
        };
        match AudioOutput::start(engine, &want, self.shared.clone()) {
            Ok(out) => {
                let device = out.info.device.clone();
                self.started(out, now);
                self.lost = None;
                Some(
                    msg("notice.audio.output_back")
                        .arg("device", device_label(device.as_deref()))
                        .into(),
                )
            }
            Err((e, engine)) => {
                // Listed but won't open (busy, unsupported rate): back to the fallback,
                // and wait longer before the next try.
                tracing::warn!(%e, "audio device is listed again but failed to open");
                let _ = self.start_fallback(engine, &want, now);
                if let Some(l) = &mut self.lost {
                    l.backoff = (l.backoff * 2).min(RETRY_MAX);
                    l.next_probe = now + l.backoff;
                }
                None
            }
        }
    }

    /// After a start that wanted `run`: a fallback (no device at startup, or when the
    /// device is opened again) is watched like a lost device, so audio comes back when the
    /// device does.
    pub(super) fn watch_fallback(&mut self, run: &AudioSettings, now: Instant) {
        let fell_back = run.backend == AudioBackendKind::Cpal
            && self
                .output
                .as_ref()
                .is_none_or(|o| o.info.backend != run.backend);
        self.lost = fell_back.then(|| Lost::new(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::AudioShared;
    use ether_core::protocol::engine::EngineEvent;
    use ether_core::protocol::message::Event;
    use std::sync::Arc;

    fn null_state() -> AudioState {
        let settings = AudioSettings {
            backend: AudioBackendKind::Null,
            max_block_size: 256,
            ..Default::default()
        };
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: 48_000,
            max_block_size: 256,
            ..Default::default()
        });
        let shared = Arc::new(AudioShared::default());
        let out = AudioOutput::start(Box::new(parts.engine), &settings, shared.clone())
            .map_err(|(e, _)| e)
            .unwrap();
        AudioState::new(
            Some(out),
            None,
            settings,
            None,
            shared,
            48_000,
            std::env::temp_dir(),
            "device-test".into(),
        )
    }

    fn notifications(msgs: &[ServerMessage]) -> Vec<(NotificationLevel, String)> {
        msgs.iter()
            .filter_map(|m| match m {
                ServerMessage::Event(Event::Notification { level, message, .. }) => {
                    Some((*level, message.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn statuses(msgs: &[ServerMessage]) -> Vec<bool> {
        msgs.iter()
            .filter_map(|m| match m {
                ServerMessage::Event(Event::Engine {
                    event: EngineEvent::Status { status },
                }) => Some(status.running),
                _ => None,
            })
            .collect()
    }

    fn frames_advance(state: &AudioState) -> bool {
        let before = state.shared.frames.load(Ordering::Relaxed);
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
            if state.shared.frames.load(Ordering::Relaxed) > before {
                return true;
            }
        }
        false
    }

    /// The device dies mid-stream (the error callback's `DeviceNotAvailable`): exactly one
    /// warning, a `running: false` status, and the engine runs again after the tick.
    #[test]
    fn a_lost_device_is_reported_once_and_restarted() {
        let mut state = null_state();
        let now = Instant::now();
        assert!(state.check_device(now).is_empty());
        crate::audio::on_stream_error(&state.shared, &cpal::StreamError::DeviceNotAvailable);
        crate::audio::on_stream_error(&state.shared, &cpal::StreamError::DeviceNotAvailable);
        assert!(!state.status().running);
        let msgs = state.check_device(now);
        let notes = notifications(&msgs);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].0, NotificationLevel::Warning);
        assert!(notes[0].1.contains("disconnected"), "{}", notes[0].1);
        assert_eq!(statuses(&msgs), vec![false, true]);
        assert!(state.status().running);
        assert!(state.lost.is_none());
        assert!(frames_advance(&state));
        // Handled: the next tick is quiet.
        assert!(state.check_device(now).is_empty());
    }

    /// A render panic is restarted once; a crash loop stops audio (reported, not silent).
    #[test]
    fn a_crashed_render_is_restarted_then_stopped_if_it_loops() {
        let mut state = null_state();
        let now = Instant::now();
        state.shared.render_crashed.store(true, Ordering::SeqCst);
        let msgs = state.check_device(now);
        assert_eq!(notifications(&msgs).len(), 1);
        assert_eq!(statuses(&msgs), vec![false, true]);
        assert!(frames_advance(&state));
        state.shared.render_crashed.store(true, Ordering::SeqCst);
        let msgs = state.check_device(now + Duration::from_secs(1));
        let notes = notifications(&msgs);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, NotificationLevel::Error);
        assert_eq!(statuses(&msgs), vec![false, false]);
        assert!(state.parked.is_some());
    }

    /// On a fallback, the chosen device is looked for again and switched back to.
    #[test]
    fn switches_back_when_the_device_returns() {
        let mut state = null_state();
        let now = Instant::now();
        state.lost = Some(Lost::new(now));
        assert!(state.check_device(now).is_empty(), "not due yet");
        let msgs = state.check_device(now + DEVICE_PROBE);
        let notes = notifications(&msgs);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, NotificationLevel::Info);
        assert!(state.lost.is_none());
        assert!(state.status().running);
        assert!(frames_advance(&state));
    }

    /// Released (no local engine): nothing to lose.
    #[test]
    fn released_device_is_not_checked() {
        let mut state = null_state();
        state.released = true;
        state.shared.device_lost.store(true, Ordering::SeqCst);
        assert!(state.check_device(Instant::now()).is_empty());
    }
}
