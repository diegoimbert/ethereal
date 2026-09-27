//! Document commands → ops.
//!
//! Every handler reads the live project through [`DocCtx::tx`] (which already reflects the
//! ops applied earlier in the same command/batch) and applies ops through it. Validation is
//! mostly left to the model (`Project::apply` enforces every invariant); handlers add the
//! checks that need command context (clamping, "already exists" idempotency, kinds).
//!
//! **Idempotent creates**: every command that creates an entity with a client-chosen id
//! succeeds without changes if that id already exists (a retried message is harmless).

mod automation;
mod clips;
mod devices;
mod misc;
mod mixer;
mod notes;
mod tracks;

use ether_core::protocol::Command;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};

use crate::tx::{CmdResult, Tx, invalid, not_found, unsupported};

pub(crate) use clips::clip_start;
pub(crate) use misc::{MAX_BPM, MIN_BPM};

/// What document commands need from the engine side.
pub(crate) trait DocHost {
    /// Descriptor of a device (built-ins: static; plugins: the instantiated plugin's).
    fn descriptor(&mut self, device: DeviceId, kind: &DeviceKind) -> Option<DeviceDescriptor>;
    /// Live state blob of a plugin device (to keep it across remove/undo, duplicate, ...).
    fn plugin_state(&mut self, device: DeviceId) -> Option<Base64Bytes>;
    /// Instantiate a plugin for a new device right away (so its name/params are known).
    fn instantiate_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
    ) -> CmdResult<Option<DeviceDescriptor>>;
}

pub(crate) struct DocCtx<'a, 'p> {
    pub tx: Tx<'p>,
    pub ids: &'a mut IdGen,
    pub now: u64,
    /// Current playhead (for "tempo at the playhead" commands).
    pub position: Beats,
    pub host: &'a mut dyn DocHost,
    /// User-facing warnings, emitted as notifications if the edit commits.
    pub warnings: Vec<String>,
}

impl DocCtx<'_, '_> {
    pub fn p(&self) -> &Project {
        self.tx.p()
    }

    pub fn new_id<I: Id>(&mut self) -> I {
        self.ids.next(self.now)
    }

    pub fn track(&self, id: TrackId) -> CmdResult<Track> {
        self.p()
            .tracks
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("track {id}")))
    }

    pub fn clip(&self, id: ClipId) -> CmdResult<Clip> {
        self.p()
            .clips
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("clip {id}")))
    }

    pub fn device(&self, id: DeviceId) -> CmdResult<Device> {
        self.p()
            .devices
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("device {id}")))
    }

    pub fn send(&self, id: SendId) -> CmdResult<TrackSend> {
        self.p()
            .sends
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("send {id}")))
    }

    pub fn lane(&self, id: AutomationLaneId) -> CmdResult<AutomationLane> {
        self.p()
            .automation_lanes
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("automation lane {id}")))
    }

    pub fn set_track(&mut self, id: TrackId, change: TrackChange) -> CmdResult<()> {
        self.tx.update(EntityUpdate::Track { id, change })
    }

    pub fn set_clip(&mut self, id: ClipId, change: ClipChange) -> CmdResult<()> {
        self.tx.update(EntityUpdate::Clip { id, change })
    }

    pub fn set_device(&mut self, id: DeviceId, change: DeviceChange) -> CmdResult<()> {
        self.tx.update(EntityUpdate::Device { id, change })
    }

    // ─── Cascading deletes ──────────────────────────────────────────────────────────────

    pub fn delete_lane(&mut self, id: AutomationLaneId) -> CmdResult<()> {
        let points: Vec<AutomationPointId> = self
            .p()
            .automation_points
            .values()
            .filter(|p| p.lane == id)
            .map(|p| p.id)
            .collect();
        for p in points {
            self.tx.remove(EntityKey::AutomationPoint(p))?;
        }
        self.tx.remove(EntityKey::AutomationLane(id))
    }

    pub fn delete_lanes_where(&mut self, pred: impl Fn(&AutomationLane) -> bool) -> CmdResult<()> {
        let lanes: Vec<AutomationLaneId> = self
            .p()
            .automation_lanes
            .values()
            .filter(|l| pred(l))
            .map(|l| l.id)
            .collect();
        for l in lanes {
            self.delete_lane(l)?;
        }
        Ok(())
    }

    pub fn delete_clip(&mut self, id: ClipId) -> CmdResult<()> {
        let notes: Vec<NoteId> = self
            .p()
            .notes
            .values()
            .filter(|n| n.clip == id)
            .map(|n| n.id)
            .collect();
        for n in notes {
            self.tx.remove(EntityKey::Note(n))?;
        }
        let markers: Vec<WarpMarkerId> = self
            .p()
            .warp_markers
            .values()
            .filter(|m| m.clip == id)
            .map(|m| m.id)
            .collect();
        for m in markers {
            self.tx.remove(EntityKey::WarpMarker(m))?;
        }
        self.delete_lanes_where(
            |l| matches!(l.owner, AutomationOwner::Clip { clip } if clip == id),
        )?;
        self.tx.remove(EntityKey::Clip(id))
    }

    pub fn delete_send(&mut self, id: SendId) -> CmdResult<()> {
        self.delete_lanes_where(
            |l| matches!(l.target, AutomationTarget::SendLevel { send } if send == id),
        )?;
        self.delete_mappings_where(|t| {
            matches!(t, MidiMapTarget::Param { target: AutomationTarget::SendLevel { send } } if *send == id)
        })?;
        self.tx.remove(EntityKey::Send(id))
    }

    pub fn delete_device(&mut self, id: DeviceId) -> CmdResult<()> {
        // Keep a plugin's live state in the document, so undoing the removal restores it.
        let device = self.device(id)?;
        if let DeviceKind::Plugin { plugin } = &device.kind
            && let Some(state) = self.host.plugin_state(id)
            && plugin.state.as_ref() != Some(&state)
        {
            let mut plugin = plugin.clone();
            plugin.state = Some(state);
            self.set_device(id, DeviceChange::Plugin(plugin))?;
        }
        self.delete_lanes_where(
            |l| matches!(l.target, AutomationTarget::DeviceParam { device, .. } if device == id),
        )?;
        // Roadmap v2: MIDI mappings of the device, and a drum rack's pads with their chains.
        self.delete_mappings_where(|t| {
            matches!(t, MidiMapTarget::Param { target: AutomationTarget::DeviceParam { device, .. } } if *device == id)
        })?;
        let pads: Vec<DrumPadId> = self.p().pads_of(id).iter().map(|p| p.id).collect();
        for pad in pads {
            self.delete_pad(pad)?;
        }
        self.tx.remove(EntityKey::Device(id))
    }

    /// Delete a drum pad with its device chain.
    pub fn delete_pad(&mut self, id: DrumPadId) -> CmdResult<()> {
        let devices: Vec<DeviceId> = self.p().pad_devices_of(id).iter().map(|d| d.id).collect();
        for d in devices {
            self.delete_device(d)?;
        }
        self.tx.remove(EntityKey::DrumPad(id))
    }

    pub fn delete_mappings_where(&mut self, pred: impl Fn(&MidiMapTarget) -> bool) -> CmdResult<()> {
        let ids: Vec<MidiMappingId> = self
            .p()
            .midi_mappings
            .values()
            .filter(|m| pred(&m.target))
            .map(|m| m.id)
            .collect();
        for m in ids {
            self.tx.remove(EntityKey::MidiMapping(m))?;
        }
        Ok(())
    }

    /// Delete one track (not its group children: see `tracks::delete`).
    pub fn delete_track_only(&mut self, id: TrackId) -> CmdResult<()> {
        let clips: Vec<ClipId> = self
            .p()
            .clips
            .values()
            .filter(|c| c.track == id)
            .map(|c| c.id)
            .collect();
        for c in clips {
            self.delete_clip(c)?;
        }
        // Track-chain devices (a rack's delete takes its pads and pad devices along).
        let devices: Vec<DeviceId> = self.p().devices_of(id).iter().map(|d| d.id).collect();
        for d in devices {
            self.delete_device(d)?;
        }
        // Roadmap v2: mappings targeting the track; sidechains listening to it are cut.
        self.delete_mappings_where(|t| match t {
            MidiMapTarget::Param {
                target:
                    AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track },
            }
            | MidiMapTarget::TrackMute { track }
            | MidiMapTarget::TrackSolo { track }
            | MidiMapTarget::TrackArm { track } => *track == id,
            _ => false,
        })?;
        let listeners: Vec<DeviceId> = self
            .p()
            .devices
            .values()
            .filter(|d| d.sidechain == Some(id))
            .map(|d| d.id)
            .collect();
        for d in listeners {
            self.set_device(d, DeviceChange::Sidechain(None))?;
        }
        let sends: Vec<SendId> = self
            .p()
            .sends
            .values()
            .filter(|s| s.from == id || s.to == id)
            .map(|s| s.id)
            .collect();
        for s in sends {
            self.delete_send(s)?;
        }
        self.delete_lanes_where(|l| {
            matches!(l.owner, AutomationOwner::Track { track } if track == id)
                || matches!(
                    l.target,
                    AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track }
                        if track == id
                )
        })?;
        // Re-route tracks that pointed at this one.
        let others: Vec<Track> = self
            .p()
            .tracks
            .values()
            .filter(|t| t.id != id)
            .cloned()
            .collect();
        for t in others {
            if matches!(t.output, TrackOutput::Track { track } if track == id) {
                self.set_track(t.id, TrackChange::Output(TrackOutput::Default))?;
            }
            if matches!(t.input, TrackInput::Track { track } if track == id) {
                self.set_track(t.id, TrackChange::Input(TrackInput::None))?;
            }
        }
        self.tx.remove(EntityKey::Track(id))
    }

    // ─── Deep copies ────────────────────────────────────────────────────────────────────

    /// Copy a clip with its notes, warp markers and clip envelopes (new child ids).
    pub fn copy_clip(
        &mut self,
        src: &Clip,
        new_id: ClipId,
        edit: impl FnOnce(&mut Clip),
        retarget: &dyn Fn(&AutomationTarget) -> Option<AutomationTarget>,
    ) -> CmdResult<()> {
        let mut copy = src.clone();
        copy.id = new_id;
        edit(&mut copy);
        self.tx.insert(Entity::Clip(copy))?;
        let notes: Vec<Note> = self.p().notes_of(src.id).into_iter().cloned().collect();
        for mut n in notes {
            n.id = self.new_id();
            n.clip = new_id;
            self.tx.insert(Entity::Note(n))?;
        }
        let markers: Vec<WarpMarker> = self
            .p()
            .warp_markers_of(src.id)
            .into_iter()
            .cloned()
            .collect();
        for mut m in markers {
            m.id = self.new_id();
            m.clip = new_id;
            self.tx.insert(Entity::WarpMarker(m))?;
        }
        let lanes: Vec<AutomationLane> = self
            .p()
            .automation_lanes
            .values()
            .filter(|l| matches!(l.owner, AutomationOwner::Clip { clip } if clip == src.id))
            .cloned()
            .collect();
        for l in lanes {
            let Some(target) = retarget(&l.target) else {
                continue;
            };
            let mut lane = l.clone();
            lane.id = self.new_id();
            lane.owner = AutomationOwner::Clip { clip: new_id };
            lane.target = target;
            self.copy_lane(l.id, lane)?;
        }
        Ok(())
    }

    /// A clip moved from track `from` to `to`: its envelopes follow it. Volume/pan retarget
    /// to `to`, sends to `to`'s send to the same return; envelopes of devices (or sends)
    /// that `to` doesn't have are deleted, with a warning.
    pub fn retarget_clip_lanes(
        &mut self,
        clip: ClipId,
        from: TrackId,
        to: TrackId,
    ) -> CmdResult<()> {
        let lanes: Vec<AutomationLane> = self
            .p()
            .automation_lanes
            .values()
            .filter(|l| matches!(l.owner, AutomationOwner::Clip { clip: c } if c == clip))
            .cloned()
            .collect();
        for l in lanes {
            let p = self.p();
            let new = match l.target {
                AutomationTarget::TrackVolume { track } if track == from => {
                    Some(AutomationTarget::TrackVolume { track: to })
                }
                AutomationTarget::TrackPan { track } if track == from => {
                    Some(AutomationTarget::TrackPan { track: to })
                }
                AutomationTarget::SendLevel { send } => {
                    let ret = p.sends.get(&send).map(|s| s.to);
                    p.sends
                        .values()
                        .find(|s| s.from == to && Some(s.to) == ret)
                        .map(|s| AutomationTarget::SendLevel { send: s.id })
                }
                AutomationTarget::DeviceParam { device, .. } => p
                    .devices
                    .get(&device)
                    .filter(|d| d.track == to)
                    .map(|_| l.target),
                other => Some(other),
            };
            match new {
                Some(t) if t == l.target => {}
                Some(t) => self.replace_lane_target(&l, t)?,
                None => {
                    self.delete_lane(l.id)?;
                    let name = self
                        .p()
                        .clips
                        .get(&clip)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    self.warnings.push(format!(
                        "clip \"{name}\": an envelope was removed (its target does not exist on the new track)"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Change a lane's target (there is no target field op: re-create it with the same ids).
    fn replace_lane_target(
        &mut self,
        lane: &AutomationLane,
        target: AutomationTarget,
    ) -> CmdResult<()> {
        let points: Vec<AutomationPoint> =
            self.p().points_of(lane.id).into_iter().cloned().collect();
        for p in &points {
            self.tx.remove(EntityKey::AutomationPoint(p.id))?;
        }
        self.tx.remove(EntityKey::AutomationLane(lane.id))?;
        self.tx.insert(Entity::AutomationLane(AutomationLane {
            target,
            ..lane.clone()
        }))?;
        for p in points {
            self.tx.insert(Entity::AutomationPoint(p))?;
        }
        Ok(())
    }

    /// Track that a target belongs to (`None` = dangling).
    pub fn target_track(&self, target: &AutomationTarget) -> Option<TrackId> {
        let p = self.p();
        match *target {
            AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track } => {
                Some(track)
            }
            AutomationTarget::SendLevel { send } => p.sends.get(&send).map(|s| s.from),
            AutomationTarget::DeviceParam { device, .. } => p.devices.get(&device).map(|d| d.track),
        }
    }

    pub fn copy_lane(&mut self, src: AutomationLaneId, lane: AutomationLane) -> CmdResult<()> {
        let new_lane = lane.id;
        self.tx.insert(Entity::AutomationLane(lane))?;
        let points: Vec<AutomationPoint> = self.p().points_of(src).into_iter().cloned().collect();
        for mut p in points {
            p.id = self.new_id();
            p.lane = new_lane;
            self.tx.insert(Entity::AutomationPoint(p))?;
        }
        Ok(())
    }
}

// ─── Ordering helpers ───────────────────────────────────────────────────────────────────

/// Order key for inserting before `before` among `siblings` (sorted, excluding the moved
/// item). `before: None` = at the end.
pub(crate) fn order_before<I: PartialEq + Copy + std::fmt::Display>(
    siblings: &[(OrderKey, I)],
    before: Option<I>,
) -> CmdResult<OrderKey> {
    let (lo, hi) = match before {
        None => (siblings.last().map(|s| &s.0), None),
        Some(b) => {
            let i = siblings
                .iter()
                .position(|s| s.1 == b)
                .ok_or_else(|| invalid(format!("{b} is not a sibling")))?;
            (
                i.checked_sub(1).map(|j| &siblings[j].0),
                Some(&siblings[i].0),
            )
        }
    };
    OrderKey::try_between(lo, hi).map_err(invalid)
}

/// Order key right after `after` among `siblings` (sorted).
pub(crate) fn order_after<I: PartialEq + Copy>(
    siblings: &[(OrderKey, I)],
    after: I,
) -> CmdResult<OrderKey> {
    let i = siblings.iter().position(|s| s.1 == after);
    let lo = i.map(|i| &siblings[i].0);
    let hi = i.and_then(|i| siblings.get(i + 1)).map(|s| &s.0);
    OrderKey::try_between(lo.or(siblings.last().map(|s| &s.0)), hi).map_err(invalid)
}

// ─── Dispatch ───────────────────────────────────────────────────────────────────────────

/// `true` if `command` edits the document (undoable, allowed inside a `Batch`).
pub(crate) fn is_document_command(command: &Command, current: Option<ProjectId>) -> bool {
    use ether_core::protocol::devices::DeviceCommand as D;
    use ether_core::protocol::midi_map::MidiMapCommand as M;
    use ether_core::protocol::recording::RecordingCommand as R;
    use ether_core::protocol::transport::TransportCommand as T;
    use ether_core::protocol::warp::WarpCommand as W;
    match command {
        Command::Track(_)
        | Command::Clip(_)
        | Command::Note(_)
        | Command::Automation(_)
        | Command::Mixer(_) => true,
        Command::Device(c) => !matches!(c, D::ListBuiltin | D::GetDescriptor { .. }),
        Command::Transport(c) => matches!(
            c,
            T::SetLoopEnabled { .. }
                | T::SetLoopRegion { .. }
                | T::SetTempo { .. }
                | T::SetTimeSignature { .. }
                | T::SetMetronome { .. }
        ),
        Command::Recording(c) => matches!(
            c,
            R::SetMonitor { .. } | R::SetInput { .. } | R::SetCountIn { .. }
        ),
        Command::Warp(c) => !matches!(c, W::DetectTempo { .. }),
        // Roadmap v2.
        Command::Tempo(_)
        | Command::Marker(_)
        | Command::Groove(_)
        | Command::DrumRack(_)
        | Command::Slice(_) => true,
        Command::MidiMap(c) => !matches!(c, M::Learn { .. } | M::List),
        Command::Project(ProjectCommand::Rename { id, .. }) => Some(*id) == current,
        _ => false,
    }
}

/// Human-readable undo label ("SetVolume" → "Set Volume").
pub(crate) fn label_of(command: &Command) -> String {
    if let Command::Edit(EditCommand::Batch { label, .. }) = command {
        return label.clone();
    }
    let ty = serde_json::to_value(command)
        .ok()
        .and_then(|v| v["command"]["type"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut out = String::with_capacity(ty.len() + 4);
    let mut prev_lower = false;
    for ch in ty.chars() {
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = ch.is_lowercase();
        out.push(ch);
    }
    out
}

/// Apply one document command (see [`is_document_command`]).
pub(crate) fn apply(ctx: &mut DocCtx, command: &Command) -> CmdResult<ReplyValue> {
    match command {
        Command::Track(c) => tracks::apply(ctx, c),
        Command::Clip(c) => clips::apply(ctx, c),
        Command::Note(c) => notes::apply(ctx, c),
        Command::Automation(c) => automation::apply(ctx, c),
        Command::Mixer(c) => mixer::apply(ctx, c),
        Command::Device(c) => devices::apply(ctx, c),
        Command::Transport(c) => misc::transport(ctx, c),
        Command::Recording(c) => misc::recording(ctx, c),
        Command::Warp(c) => misc::warp(ctx, c),
        Command::Tempo(c) => crate::tempo::apply(ctx, c),
        Command::Marker(c) => crate::clip_editing::marker_command(ctx, c),
        Command::Groove(c) => crate::groove::apply(ctx, c),
        Command::DrumRack(c) => crate::drum_rack::rack_command(ctx, c),
        Command::Slice(c) => crate::drum_rack::slice_command(ctx, c),
        Command::MidiMap(c) => crate::midi_learn::apply(ctx, c),
        Command::Project(ProjectCommand::Rename { name, .. }) => misc::rename_project(ctx, name),
        other => Err(unsupported(format!(
            "{} is not a document command",
            label_of(other)
        ))),
    }?;
    Ok(ReplyValue::Unit)
}

pub(crate) use devices::builtin_descriptor;
