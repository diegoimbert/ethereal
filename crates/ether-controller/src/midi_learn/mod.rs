//! MIDI learn / controller mappings (roadmap v2, owned by the `midi-learn` node; see
//! `docs/ROADMAP.md`, CONTRACTS.md §11.9 and `ether_protocol::midi_map`).
//!
//! - Document part (`Map`/`Edit`/`Unmap`): [`apply`], dispatched from `doc::apply`. Undoable.
//! - Runtime part (`Learn`, `List`): [`EtherController::midi_map_command`]. Not undoable.
//! - Input: [`EtherController::midi_learn_tick`] drains `EngineBridge::poll_midi_input`,
//!   completes a pending learn, and turns mapped messages into ordinary commands, run through
//!   the normal dispatch (so they patch, reach the engine and validate like UI edits).
//!
//! # Learn
//! `Learn { target: Some(t) }` arms learning (`LearnChanged`). The next CC, note-on or pitch
//! bend creates the mapping for `t` in one undo step ("MIDI Learn"): it replaces any mapping
//! of `t` and any mapping of the same source. The source is concrete (the message's port and
//! channel). Learning then ends (`Learned`, then `LearnChanged { None }`). Default mode:
//! `Toggle` for notes and for on/off targets (mute, solo, arm), `Absolute` otherwise;
//! range `0..1`. The learning message itself is not applied.
//!
//! # Applying mapped input (and undo)
//! A message is applied to the most specific matching mapping (concrete port/channel beat
//! wildcards). Values are normalized like automation (`compile::track_param_info` for mixer
//! targets, the device descriptor for device params) and sent as the usual commands:
//!
//! - **Continuous targets** (device param, volume, pan, send level): `Absolute` sets
//!   `min + v·(max − min)`; `Relative` adds `steps · (max − min) / 127` to the current value
//!   (clamped to the range; notes and pitch bend fall back to absolute); `Toggle` flips
//!   between `min` and `max` on each press. Moves of one mapping share one controller-owned
//!   gesture that ends after [`GESTURE_IDLE_MS`] without messages, so a knob turn is **one
//!   undo step**, never one per message. Values equal to the current one send nothing.
//! - **On/off targets** (mute, solo): `Absolute` sets on/off from the threshold (only when
//!   the state changes), `Toggle` flips on each press, `Relative` turns on/off with the sign
//!   of the increment. One undo step per state change. Record-arm is runtime state (never
//!   undoable), as for `Recording::Arm`. Solo is not exclusive.
//! - **Transport actions** fire on each press, whatever the mode. Play/stop/record/locate
//!   are not undoable; loop and metronome toggles are one undo step each (project settings),
//!   tap tempo is merged by the tap-tempo gesture.
//!
//! A "press" is the scaled value crossing the midpoint of `min..max` upwards (note-on with
//! the default range; `min > max` inverts). The previous value is remembered per source.
//! Mapped messages are never filtered from monitored tracks (the engine gets them on its own
//! path). Mappings whose target vanished are skipped silently.
//!
//! # Host input
//! Natively, `ether-native/src/recording` queues a [`MidiInputEvent`] for every short message
//! of every connected port (ports are connected by `Recording::ListInputs`); the web host
//! has no MIDI input yet (the default `poll_midi_input`).

mod doc;
mod input;

use std::collections::{BTreeMap, HashMap};

use ether_core::protocol::devices::{DeviceCommand, ParamInfo};
use ether_core::protocol::midi_map::{MidiInputEvent, MidiMapCommand, MidiMapEvent};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{ClientMessage, Command, Event, NotificationLevel, ReplyValue};

use crate::compile::track_param_info;
use crate::doc::DocHost;
use crate::engine::EngineCtx;
use crate::handlers::{event, no_project, notify};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, internal};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

pub(crate) use doc::apply;
use input::Decoded;

/// A mapping's gesture ends after this long without messages (one undo step per movement).
pub const GESTURE_IDLE_MS: u64 = 300;
/// Minimum interval between two `Activity` events (~10 Hz).
pub const ACTIVITY_INTERVAL_MS: u64 = 100;
/// Remembered sources for press detection (bounded; cleared when exceeded).
const MAX_REMEMBERED: usize = 4096;

/// Runtime MIDI learn state (not in the document).
#[derive(Debug, Default)]
pub(crate) struct MidiLearnState {
    /// Target being learned.
    learn: Option<MidiMapTarget>,
    /// Open gesture of each mapping and the time of its last message.
    gestures: BTreeMap<MidiMappingId, (GestureId, u64)>,
    /// Last control position per concrete source (port, channel, control).
    last: HashMap<(String, u8, MidiControl), f64>,
    /// Last source seen, not yet reported.
    activity: Option<MidiSource>,
    last_activity_ms: Option<u64>,
    buf: Vec<MidiInputEvent>,
}

/// The value a mapped message resolves to.
enum Action {
    Continuous {
        target: AutomationTarget,
        info: ParamInfo,
        /// Normalized current value.
        current: f64,
        /// Plain current value.
        plain: f64,
    },
    OnOff {
        target: OnOffTarget,
        current: bool,
    },
    Transport(TransportAction),
}

#[derive(Clone, Copy)]
enum OnOffTarget {
    Mute(TrackId),
    Solo(TrackId),
    Arm(TrackId),
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `MidiMap::Learn` / `MidiMap::List` (not document commands).
    pub(crate) fn midi_map_command(
        &mut self,
        c: &MidiMapCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        match c {
            MidiMapCommand::Learn { target } => {
                self.set_learn(target.clone(), out);
                Ok(ReplyValue::Unit)
            }
            MidiMapCommand::List => {
                let mut mappings: Vec<MidiMapping> =
                    doc.project.midi_mappings.values().cloned().collect();
                mappings.sort_by_key(|m| input::source_key(&m.source));
                Ok(ReplyValue::MidiMappings { mappings })
            }
            other => Err(internal(format!("{other:?} is a document command"))),
        }
    }

    fn set_learn(&mut self, target: Option<MidiMapTarget>, out: &mut dyn MessageSink) {
        self.midi_learn.learn = target.clone();
        event(
            out,
            Event::MidiMap {
                event: MidiMapEvent::LearnChanged { target },
            },
        );
    }

    /// Called from every tick.
    pub(crate) fn midi_learn_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let mut buf = std::mem::take(&mut self.midi_learn.buf);
        self.bridge.poll_midi_input(&mut buf);
        for e in buf.drain(..) {
            self.midi_input(&e, now, out);
        }
        self.midi_learn.buf = buf;

        // Close idle gestures (one undo step per movement).
        let idle: Vec<(MidiMappingId, GestureId)> = self
            .midi_learn
            .gestures
            .iter()
            .filter(|(_, (_, last))| now.saturating_sub(*last) >= GESTURE_IDLE_MS)
            .map(|(m, (g, _))| (*m, *g))
            .collect();
        for (m, g) in idle {
            self.midi_learn.gestures.remove(&m);
            if let Some(doc) = self.doc.as_mut() {
                doc.history.end_gesture(g);
            }
        }

        // Throttled activity for UI feedback.
        let due = self
            .midi_learn
            .last_activity_ms
            .is_none_or(|t| now.saturating_sub(t) >= ACTIVITY_INTERVAL_MS || now < t);
        if due && let Some(source) = self.midi_learn.activity.take() {
            self.midi_learn.last_activity_ms = Some(now);
            event(
                out,
                Event::MidiMap {
                    event: MidiMapEvent::Activity { source },
                },
            );
        }
    }

    fn midi_input(&mut self, e: &MidiInputEvent, now: u64, out: &mut dyn MessageSink) {
        let Some(d) = input::decode(e.data) else {
            return;
        };
        self.midi_learn.activity = Some(MidiSource {
            port: Some(e.port.clone()),
            channel: Some(d.channel),
            control: d.control,
        });
        if self.doc.is_none() {
            return;
        }
        let key = (e.port.clone(), d.channel, d.control);
        if self.midi_learn.last.len() >= MAX_REMEMBERED && !self.midi_learn.last.contains_key(&key)
        {
            self.midi_learn.last.clear();
        }
        let previous = self.midi_learn.last.insert(key, d.value).unwrap_or(0.0);

        if self.midi_learn.learn.is_some() {
            if matches!(d.control, MidiControl::Note { .. }) && !d.note_on {
                return;
            }
            self.complete_learn(&e.port, &d, now, out);
            return;
        }

        let Some(mapping) = self.doc.as_ref().and_then(|doc| {
            doc.project
                .midi_mappings
                .values()
                .filter(|m| input::matches(&m.source, &e.port, &d))
                .max_by_key(|m| input::specificity(&m.source))
                .cloned()
        }) else {
            return;
        };
        self.apply_mapping(&mapping, &d, previous, now, out);
    }

    fn complete_learn(&mut self, port: &str, d: &Decoded, now: u64, out: &mut dyn MessageSink) {
        let Some(target) = self.midi_learn.learn.clone() else {
            return;
        };
        let source = MidiSource {
            port: Some(port.to_string()),
            channel: Some(d.channel),
            control: d.control,
        };
        let on_off = matches!(
            target,
            MidiMapTarget::TrackMute { .. }
                | MidiMapTarget::TrackSolo { .. }
                | MidiMapTarget::TrackArm { .. }
        );
        let mode = if on_off || matches!(d.control, MidiControl::Note { .. }) {
            MidiMapMode::Toggle
        } else {
            MidiMapMode::Absolute
        };
        let mapping = MidiMapping {
            id: self.ids.next(now),
            source,
            target,
            min: 0.0,
            max: 1.0,
            mode,
        };
        let id = mapping.id;
        let result = self.edit_with("MIDI Learn", None, now, out, |ctx| {
            doc::learn(ctx, mapping)
        });
        match result {
            Ok(()) => event(
                out,
                Event::MidiMap {
                    event: MidiMapEvent::Learned { mapping: id },
                },
            ),
            Err(e) => notify(
                out,
                NotificationLevel::Warning,
                format!("MIDI learn failed: {}", e.message),
            ),
        }
        self.set_learn(None, out);
    }

    /// Resolve a mapping's target against the current document (`None` = gone).
    fn resolve(&mut self, target: &MidiMapTarget) -> Option<Action> {
        let doc = self.doc.as_ref()?;
        let p = &doc.project;
        Some(match target {
            MidiMapTarget::Param { target } => {
                let (info, plain) = match *target {
                    AutomationTarget::TrackVolume { track } => (
                        track_param_info(target)?,
                        f64::from(p.tracks.get(&track)?.mixer.volume.0),
                    ),
                    AutomationTarget::TrackPan { track } => (
                        track_param_info(target)?,
                        f64::from(p.tracks.get(&track)?.mixer.pan.0),
                    ),
                    AutomationTarget::SendLevel { send } => (
                        track_param_info(target)?,
                        f64::from(p.sends.get(&send)?.level.0),
                    ),
                    AutomationTarget::DeviceParam { device, param } => {
                        let d = p.devices.get(&device)?.clone();
                        let mut host = EngineCtx {
                            bridge: &mut self.bridge,
                            eng: &mut self.engine,
                        };
                        let desc = host.descriptor(d.id, &d.kind)?;
                        let info = desc.params.into_iter().find(|i| i.id == param)?;
                        let plain = d.params.get(&param).copied().unwrap_or(info.default);
                        (info, plain)
                    }
                };
                Action::Continuous {
                    target: *target,
                    current: info.to_normalized(plain),
                    plain,
                    info,
                }
            }
            MidiMapTarget::TrackMute { track } => Action::OnOff {
                target: OnOffTarget::Mute(*track),
                current: p.tracks.get(track)?.mixer.mute,
            },
            MidiMapTarget::TrackSolo { track } => Action::OnOff {
                target: OnOffTarget::Solo(*track),
                current: p.tracks.get(track)?.mixer.solo,
            },
            MidiMapTarget::TrackArm { track } => {
                p.tracks.get(track)?;
                Action::OnOff {
                    target: OnOffTarget::Arm(*track),
                    current: self.armed.contains(track),
                }
            }
            MidiMapTarget::Transport { action } => Action::Transport(*action),
        })
    }

    fn apply_mapping(
        &mut self,
        m: &MidiMapping,
        d: &Decoded,
        previous: f64,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let pressed = input::is_on(m, d.value) && !input::is_on(m, previous);
        let relative = match m.mode {
            MidiMapMode::Relative { encoding } if matches!(d.control, MidiControl::Cc { .. }) => {
                Some(input::relative_steps(d.raw, encoding))
            }
            _ => None,
        };
        let Some(action) = self.resolve(&m.target) else {
            return;
        };
        match action {
            Action::Continuous {
                target,
                info,
                current,
                plain: current_plain,
            } => {
                let n = match (m.mode, relative) {
                    (_, Some(steps)) => input::relative_target(m, current, steps),
                    (MidiMapMode::Toggle, _) if pressed => input::toggled(m, current),
                    (MidiMapMode::Toggle, _) => return,
                    _ => input::scaled(m, d.value),
                };
                let plain = info.to_plain(n);
                if (plain - current_plain).abs() < 1e-6 * current_plain.abs().max(1.0) {
                    return;
                }
                let command = match target {
                    AutomationTarget::TrackVolume { track } => {
                        Command::Mixer(MixerCommand::SetVolume {
                            track,
                            volume: Decibels(plain as f32),
                        })
                    }
                    AutomationTarget::TrackPan { track } => Command::Mixer(MixerCommand::SetPan {
                        track,
                        pan: Pan(plain as f32),
                    }),
                    AutomationTarget::SendLevel { send } => {
                        Command::Mixer(MixerCommand::SetSendLevel {
                            send,
                            level: Decibels(plain as f32),
                        })
                    }
                    AutomationTarget::DeviceParam { device, param } => {
                        Command::Device(DeviceCommand::SetParam {
                            device,
                            param,
                            value: plain,
                        })
                    }
                };
                let gesture = match self.midi_learn.gestures.get(&m.id) {
                    Some((g, _)) => *g,
                    None => self.new_gesture(),
                };
                self.midi_learn.gestures.insert(m.id, (gesture, now));
                self.run(command, Some(gesture), now, out);
            }
            Action::OnOff { target, current } => {
                let want = match (m.mode, relative) {
                    (_, Some(0)) => return,
                    (_, Some(steps)) => steps > 0,
                    (MidiMapMode::Toggle, _) if pressed => !current,
                    (MidiMapMode::Toggle, _) => return,
                    _ => input::is_on(m, d.value),
                };
                if want == current {
                    return;
                }
                let command = match target {
                    OnOffTarget::Mute(track) => Command::Mixer(MixerCommand::SetMute {
                        track,
                        mute: want,
                    }),
                    OnOffTarget::Solo(track) => Command::Mixer(MixerCommand::SetSolo {
                        track,
                        solo: want,
                        exclusive: false,
                    }),
                    OnOffTarget::Arm(track) => Command::Recording(RecordingCommand::Arm {
                        track,
                        armed: want,
                        exclusive: false,
                    }),
                };
                self.run(command, None, now, out);
            }
            Action::Transport(action) => {
                if pressed && let Some(command) = self.transport_action(action) {
                    self.run(command, None, now, out);
                }
            }
        }
    }

    fn transport_action(&self, action: TransportAction) -> Option<Command> {
        let settings = &self.doc.as_ref()?.project.settings;
        Some(match action {
            TransportAction::Play => Command::Transport(TransportCommand::Play),
            TransportAction::Stop => Command::Transport(TransportCommand::Stop),
            TransportAction::TogglePlay => Command::Transport(TransportCommand::TogglePlay),
            TransportAction::TapTempo => Command::Transport(TransportCommand::TapTempo),
            TransportAction::ToggleRecord => Command::Recording(RecordingCommand::SetRecording {
                enabled: !self.transport.recording,
            }),
            TransportAction::ToggleLoop => Command::Transport(TransportCommand::SetLoopEnabled {
                enabled: !settings.loop_enabled,
            }),
            TransportAction::ToggleMetronome => {
                Command::Transport(TransportCommand::SetMetronome {
                    enabled: !settings.metronome,
                })
            }
            TransportAction::PreviousMarker | TransportAction::NextMarker => {
                let at = self.transport.position.0;
                const EPS: f64 = 1e-6;
                let positions = self
                    .doc
                    .as_ref()?
                    .project
                    .markers
                    .values()
                    .map(|m| m.position.0);
                let position = if matches!(action, TransportAction::NextMarker) {
                    positions.filter(|p| *p > at + EPS).min_by(f64::total_cmp)?
                } else {
                    positions.filter(|p| *p < at - EPS).max_by(f64::total_cmp)?
                };
                Command::Transport(TransportCommand::Locate {
                    position: Beats(position),
                })
            }
        })
    }

    /// Run a mapped command through the normal dispatch (no reply is sent).
    fn run(
        &mut self,
        command: Command,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let msg = ClientMessage {
            id: 0,
            gesture,
            command,
        };
        // A stale or invalid target (e.g. arming a return track) is ignored.
        let _ = self.dispatch(&msg, now, out);
    }
}
