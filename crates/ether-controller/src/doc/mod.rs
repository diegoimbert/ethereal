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
mod labels;
mod misc;
mod mixer;
mod notes;
mod reverse;
mod stretch;
mod tracks;

use ether_core::protocol::Command;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;

use crate::tx::{CmdResult, Tx, invalid, not_found, unsupported};

pub use clips::{MIN_CLIP_SLIVER, clip_start, drop_slivers};
pub use labels::UndoLabel;
pub use labels::{ExtLabels, set_ext_labels};
pub use labels::{label_of, undo};
pub use misc::{MAX_BPM, MIN_BPM};
pub use reverse::reverse_envelopes;
pub(crate) use tracks::order_at_end;

/// What document commands need from the engine side.
pub trait DocHost {
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
    /// Whether a new instance of this plugin starts sandboxed (`sandboxed: null` inserts;
    /// [`crate::EngineBridge::sandbox_by_default`]). Default: no.
    fn sandbox_by_default(&self, format: PluginFormat, plugin_id: &str) -> bool {
        let _ = (format, plugin_id);
        false
    }
    /// The flag of a new instance: `explicit` when the command gave one, else the default.
    fn new_instance_sandboxed(
        &self,
        explicit: Option<bool>,
        format: PluginFormat,
        plugin_id: &str,
    ) -> bool {
        explicit.unwrap_or_else(|| self.sandbox_by_default(format, plugin_id))
    }
    /// Roadmap v2 (`drum-rack`): waveform peaks of loaded media (auto-slicing by
    /// transients). `None` = not loaded (yet).
    fn peaks(&self, media: MediaId) -> Option<&ether_media::PeakMipmap> {
        let _ = media;
        None
    }
    /// Roadmap v2 (`drum-rack`): runtime pad solo (`DrumRack::SetPadSolo`; not a document
    /// change, compiled as mute of the rack's other pads).
    fn set_pad_solo(&mut self, pad: DrumPadId, solo: bool) {
        let _ = (pad, solo);
    }
    /// v0.2 (`comping`): runtime take audition (`Take::Audition`; not a document change).
    fn set_audition(&mut self, track: TrackId, lane: Option<TakeLaneId>) {
        let _ = (track, lane);
    }
}

pub struct DocCtx<'a, 'p> {
    pub tx: Tx<'p>,
    pub ids: &'a mut IdGen,
    pub now: u64,
    /// Current playhead (for "tempo at the playhead" commands).
    pub position: Beats,
    pub host: &'a mut dyn DocHost,
    /// User-facing warnings, emitted as notifications if the edit commits.
    pub warnings: Vec<crate::text::Msg>,
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

    /// Write plugin device `id`'s running state into the document directly, outside the
    /// transaction's ops (like a save, never undone; see `keep_live_state` for preset load
    /// and Replace). The undo of an edit that removes or unloads the instance then puts back
    /// what was running, not the last saved state. Returns the state if it changed.
    pub fn keep_running_plugin_state(&mut self, id: DeviceId) -> Option<Base64Bytes> {
        let state = self.host.plugin_state(id)?;
        match self.tx.project.devices.get_mut(&id).map(|d| &mut d.kind) {
            Some(DeviceKind::Plugin { plugin }) if plugin.state.as_ref() != Some(&state) => {
                plugin.state = Some(state.clone());
                Some(state)
            }
            _ => None,
        }
    }

    // ─── Cascading deletes ──────────────────────────────────────────────────────────────

    pub fn delete_lane(&mut self, id: AutomationLaneId) -> CmdResult<()> {
        let points = self.p().point_ids_of(id);
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
        // Indexed lookups: a clip's children without scanning the tables.
        for n in self.p().note_ids_of(id) {
            self.delete_note(n)?;
        }
        // v0.3: expression lanes.
        for l in self.p().expression_lane_ids_of(id) {
            self.tx.remove(EntityKey::ExpressionLane(l))?;
        }
        for m in self.p().warp_marker_ids_of(id) {
            self.tx.remove(EntityKey::WarpMarker(m))?;
        }
        for l in self
            .p()
            .lane_ids_owned_by(AutomationOwner::Clip { clip: id })
        {
            self.delete_lane(l)?;
        }
        self.tx.remove(EntityKey::Clip(id))
    }

    /// Remove a note and its expressions (v0.3).
    pub fn delete_note(&mut self, id: NoteId) -> CmdResult<()> {
        for e in self.p().note_expression_ids_of(id) {
            self.tx.remove(EntityKey::NoteExpression(e))?;
        }
        self.tx.remove(EntityKey::Note(id))
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
        // Keep a plugin's live state in the document (outside the history), so the device
        // that undoing the removal puts back carries it.
        self.device(id)?;
        self.keep_running_plugin_state(id);
        self.delete_lanes_where(|l| l.target.device() == Some(id))?;
        // Roadmap v2: MIDI mappings of the device, and a drum rack's pads with their chains.
        self.delete_mappings_where(
            |t| matches!(t, MidiMapTarget::Param { target } if target.device() == Some(id)),
        )?;
        let pads: Vec<DrumPadId> = self.p().pads_of(id).iter().map(|p| p.id).collect();
        for pad in pads {
            self.delete_pad(pad)?;
        }
        // v0.2: modulation mappings targeting the device or sourced from its modulators /
        // macros, its modulators, and a rack's chains with their devices.
        let mappings: Vec<ModMappingId> = self
            .p()
            .mod_mappings
            .values()
            .filter(|m| {
                m.device == id
                    || match m.source {
                        ModSource::Macro { rack, .. } => rack == id,
                        ModSource::Modulator { modulator } => self
                            .p()
                            .modulators
                            .get(&modulator)
                            .is_some_and(|x| x.device == id),
                    }
            })
            .map(|m| m.id)
            .collect();
        for m in mappings {
            self.tx.remove(EntityKey::ModMapping(m))?;
        }
        let modulators: Vec<ModulatorId> =
            self.p().modulators_of(id).iter().map(|m| m.id).collect();
        for m in modulators {
            self.tx.remove(EntityKey::Modulator(m))?;
        }
        let chains: Vec<RackChainId> = self.p().chains_of(id).iter().map(|c| c.id).collect();
        for c in chains {
            self.delete_rack_chain(c)?;
        }
        self.tx.remove(EntityKey::Device(id))
    }

    /// Cut the sidechains tapping chain `chain` (before it goes).
    fn cut_chain_listeners(&mut self, chain: SidechainChain) -> CmdResult<()> {
        let listeners: Vec<Device> = self
            .p()
            .devices
            .values()
            .filter(|d| d.sidechain_tap.and_then(|t| t.chain) == Some(chain))
            .cloned()
            .collect();
        for d in listeners {
            crate::sidechain::cut(self, &d)?;
        }
        Ok(())
    }

    /// Delete a rack chain with its devices (v0.2).
    pub fn delete_rack_chain(&mut self, id: RackChainId) -> CmdResult<()> {
        self.cut_chain_listeners(SidechainChain::Rack { chain: id })?;
        let devices: Vec<DeviceId> = self.p().chain_devices_of(id).iter().map(|d| d.id).collect();
        for d in devices {
            self.delete_device(d)?;
        }
        self.tx.remove(EntityKey::RackChain(id))
    }

    /// Copy the pads of drum rack `src` (with their device chains) onto rack `dst` on track
    /// `track`, with new ids. `device_ids` receives old → new pad-device ids (for
    /// retargeting automation). Used by device and track duplication.
    pub fn copy_rack_pads(
        &mut self,
        src: DeviceId,
        dst: DeviceId,
        track: TrackId,
        device_ids: &mut std::collections::BTreeMap<DeviceId, DeviceId>,
    ) -> CmdResult<()> {
        let pads: Vec<DrumPad> = self.p().pads_of(src).into_iter().cloned().collect();
        for pad in pads {
            let mut np = pad.clone();
            np.id = self.new_id();
            np.rack = dst;
            let new_pad = np.id;
            self.tx.insert(Entity::DrumPad(np))?;
            let devices: Vec<Device> = self
                .p()
                .pad_devices_of(pad.id)
                .into_iter()
                .cloned()
                .collect();
            for d in devices {
                let mut nd = d.clone();
                nd.id = self.new_id();
                nd.track = track;
                nd.pad = Some(new_pad);
                if let DeviceKind::Plugin { plugin } = &mut nd.kind
                    && let Some(state) = self.host.plugin_state(d.id)
                {
                    plugin.state = Some(state);
                }
                device_ids.insert(d.id, nd.id);
                self.tx.insert(Entity::Device(nd))?;
            }
        }
        Ok(())
    }

    /// Delete a drum pad with its device chain.
    pub fn delete_pad(&mut self, id: DrumPadId) -> CmdResult<()> {
        self.cut_chain_listeners(SidechainChain::Pad { pad: id })?;
        let devices: Vec<DeviceId> = self.p().pad_devices_of(id).iter().map(|d| d.id).collect();
        for d in devices {
            self.delete_device(d)?;
        }
        self.tx.remove(EntityKey::DrumPad(id))
    }

    pub fn delete_mappings_where(
        &mut self,
        pred: impl Fn(&MidiMapTarget) -> bool,
    ) -> CmdResult<()> {
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
        // v0.2: comp regions before the lanes they select, lane clips with the other clips,
        // then the lanes.
        let regions: Vec<CompRegionId> = self.p().comp_of(id).iter().map(|r| r.id).collect();
        for r in regions {
            self.tx.remove(EntityKey::CompRegion(r))?;
        }
        for c in self.p().clip_ids_of(id) {
            self.delete_clip(c)?;
        }
        let lanes: Vec<TakeLaneId> = self.p().lanes_of(id).iter().map(|l| l.id).collect();
        for l in lanes {
            self.tx.remove(EntityKey::TakeLane(l))?;
        }
        // Sidechains listening to it are cut first (they may tap its chains, which go
        // with its devices).
        let listeners: Vec<Device> = self
            .p()
            .devices
            .values()
            .filter(|d| d.sidechain == Some(id) && d.track != id)
            .cloned()
            .collect();
        for d in listeners {
            crate::sidechain::cut(self, &d)?;
        }
        let midi_listeners: Vec<Device> = self
            .p()
            .devices
            .values()
            .filter(|d| d.midi_sidechain.is_some_and(|m| m.track == id) && d.track != id)
            .cloned()
            .collect();
        for d in midi_listeners {
            crate::midi_sidechain::cut(self, &d)?;
        }
        // Track-chain devices (a rack's delete takes its pads and pad devices along).
        let devices: Vec<DeviceId> = self.p().devices_of(id).iter().map(|d| d.id).collect();
        for d in devices {
            self.delete_device(d)?;
        }
        // Roadmap v2: mappings targeting the track.
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
        // v0.2: envelope followers listening to it too.
        let followers: Vec<ModulatorId> = self
            .p()
            .modulators
            .values()
            .filter(|m| m.sidechain == Some(id))
            .map(|m| m.id)
            .collect();
        for m in followers {
            self.tx.update(EntityUpdate::Modulator {
                id: m,
                change: ModulatorChange::Sidechain(None),
            })?;
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
            if matches!(t.input, TrackInput::Track { track, .. } if track == id) {
                self.set_track(t.id, TrackChange::Input(TrackInput::None))?;
            }
            // v0.2: tracks assigned to a deleted VCA are unassigned.
            if t.vca == Some(id) {
                self.set_track(t.id, TrackChange::Vca(None))?;
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
            let old = n.id;
            n.id = self.new_id();
            n.clip = new_id;
            let new_note = n.id;
            self.tx.insert(Entity::Note(n))?;
            // v0.3: the note's expressions follow it.
            let exprs: Vec<NoteExpression> = self
                .p()
                .note_expressions_of(old)
                .into_iter()
                .cloned()
                .collect();
            for mut e in exprs {
                e.id = self.new_id();
                e.note = new_note;
                self.tx.insert(Entity::NoteExpression(e))?;
            }
        }
        // v0.3: clip expression lanes.
        let lanes: Vec<ExpressionLane> = self
            .p()
            .expression_lanes_of(src.id)
            .into_iter()
            .cloned()
            .collect();
        for mut l in lanes {
            l.id = self.new_id();
            l.clip = new_id;
            self.tx.insert(Entity::ExpressionLane(l))?;
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
                AutomationTarget::DeviceParam { device, .. }
                | AutomationTarget::DeviceEnabled { device } => p
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
                    self.warnings.push(
                        crate::text::msg("notice.clip.envelope_removed")
                            .arg("clip", name)
                            .into(),
                    );
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
            AutomationTarget::DeviceParam { device, .. }
            | AutomationTarget::DeviceEnabled { device } => p.devices.get(&device).map(|d| d.track),
            // The owning clip's own property: no track of its own.
            AutomationTarget::ClipGain | AutomationTarget::ClipTranspose => None,
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
pub fn order_before<I: PartialEq + Copy + std::fmt::Display>(
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
pub fn order_after<I: PartialEq + Copy>(
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
pub fn is_document_command(command: &Command, current: Option<ProjectId>) -> bool {
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
        Command::Device(c) => !matches!(
            c,
            D::ListBuiltin | D::GetDescriptor { .. } | D::ListFactoryIrs
        ),
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
        // v0.2.
        Command::Take(_) | Command::Rack(_) => true,
        Command::Modulation(c) => !matches!(
            c,
            ether_core::protocol::racks::ModulationCommand::ListModulatorKinds
        ),
        // v0.3.
        Command::Expression(_) => true,
        Command::External(c) => matches!(
            c,
            ether_core::protocol::external::ExternalCommand::SetRouting { .. }
        ),
        Command::Template(c) => matches!(
            c,
            ether_core::protocol::templates::TemplateCommand::Insert { .. }
        ),
        Command::Project(ProjectCommand::SetScale { .. }) => true,
        Command::Project(ProjectCommand::Rename { id, .. }) => Some(*id) == current,
        // An extension's document commands (`set_ext_doc_commands`).
        other => EXT_DOC.get().is_some_and(|x| (x.is_document)(other)),
    }
}

/// An extension's document commands: which commands are document commands (applied in a
/// transaction, one undo step), and how they apply to the document.
#[derive(Clone, Copy)]
pub struct ExtDocCommands {
    pub is_document: fn(&Command) -> bool,
    /// `None`: not the extension's.
    pub apply: fn(&mut DocCtx, &Command) -> Option<CmdResult<()>>,
}

static EXT_DOC: std::sync::OnceLock<ExtDocCommands> = std::sync::OnceLock::new();

/// Extension point: an extension's document commands (set once, at start).
pub fn set_ext_doc_commands(commands: ExtDocCommands) {
    let _ = EXT_DOC.set(commands);
}

/// Apply one document command (see [`is_document_command`]).
pub fn apply(ctx: &mut DocCtx, command: &Command) -> CmdResult<ReplyValue> {
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
        Command::Take(c) => crate::comping::take_command(ctx, c),
        Command::Rack(c) => crate::racks::rack_command(ctx, c),
        Command::Modulation(c) => crate::racks::modulation_command(ctx, c),
        // v0.3.
        Command::Expression(c) => crate::expression::expression_command(ctx, c),
        Command::External(ether_core::protocol::external::ExternalCommand::SetRouting {
            device,
            routing,
        }) => crate::external::set_routing(ctx, *device, routing),
        Command::Template(ether_core::protocol::templates::TemplateCommand::Insert {
            template,
            seed,
            parent,
            before,
        }) => crate::templates::insert(ctx, template, *seed, *parent, *before),
        Command::Project(ProjectCommand::SetScale { scale }) => {
            ctx.tx.settings(SettingsChange::Scale(*scale))
        }
        Command::Project(ProjectCommand::Rename { name, .. }) => misc::rename_project(ctx, name),
        other => match EXT_DOC.get().and_then(|x| (x.apply)(ctx, other)) {
            Some(result) => result,
            None => Err(unsupported(format!(
                "{} is not a document command",
                label_of(other, None)
            ))),
        },
    }?;
    Ok(ReplyValue::Unit)
}

pub use devices::{builtin_descriptor, default_device_name, missing_descriptor};
