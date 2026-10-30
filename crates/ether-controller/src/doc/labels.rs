//! Undo labels: the short, human name of what a command does ("move 3 clips", "set
//! volume", "add reverb"), shown in the undo history panel and as "Undo …" in the UI.
//!
//! Style: lowercase (acronyms too: midi, mpe, vca, ir), a verb first, counts
//! when a command acts on several things, and the device/parameter name when the project
//! has it at hand. Every command variant is matched explicitly (no wildcard arm), so a new
//! command doesn't compile until it has a label; `tests::every_command_has_a_human_label`
//! checks every variant's label.

use ether_core::protocol::Command;
use ether_core::protocol::agent::AgentCommand;
use ether_core::protocol::audio_to_midi::AudioToMidiCommand;
use ether_core::protocol::automation::AutomationCommand as Au;
use ether_core::protocol::capture::CaptureCommand;
use ether_core::protocol::clips::ClipCommand as C;
use ether_core::protocol::devices::{DeviceCommand as D, DeviceSpec};
use ether_core::protocol::drum_rack::{DrumRackCommand as Dr, SliceCommand as Sl};
use ether_core::protocol::expression::ExpressionCommand as Ex;
use ether_core::protocol::ext::ExtCommand;
use ether_core::protocol::external::ExternalCommand;
use ether_core::protocol::freeze::FreezeCommand as F;
use ether_core::protocol::groove::GrooveCommand;
use ether_core::protocol::markers::MarkerCommand as Mk;
use ether_core::protocol::media::MediaCommand as Me;
use ether_core::protocol::media_refs::MediaRefCommand as Mr;
use ether_core::protocol::midi_map::MidiMapCommand as Mm;
use ether_core::protocol::mixer::MixerCommand as Mx;
use ether_core::protocol::model::text::{TextArg, TextKey};
use ether_core::protocol::model::{
    DeviceId, DeviceKind, ModulatorKind, ParamId, Project, TrackKind,
};
use ether_core::protocol::notes::NoteCommand as N;
use ether_core::protocol::plugins::PluginCommand as Pl;
use ether_core::protocol::presets::PresetCommand as Pr;
use ether_core::protocol::project::{EditCommand as E, ProjectCommand as P};
use ether_core::protocol::racks::{ModulationCommand as Mo, RackCommand as R};
use ether_core::protocol::recording::RecordingCommand as Rec;
use ether_core::protocol::resolver::ResolveCommand;
use ether_core::protocol::takes::TakeCommand as Tk;
use ether_core::protocol::templates::TemplateCommand as Tpl;
use ether_core::protocol::tempo::TempoCommand as Te;
use ether_core::protocol::time_edit::TimeEditCommand as Ti;
use ether_core::protocol::tracks::TrackCommand as T;
use ether_core::protocol::transport::TransportCommand as Tr;
use ether_core::protocol::warp::WarpCommand as W;

/// An undo label: the English text (agents, logs, the fallback) and, when it is one of the
/// catalog's, its message, which the UI shows in the user's language (docs/LOCALIZATION.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoLabel {
    pub text: String,
    pub key: Option<TextKey>,
}

impl UndoLabel {
    /// A `Batch`'s label: its key when the catalog has it, else the text (keyed when it is
    /// one of the catalog's messages without arguments).
    pub fn batch(text: &str, key: Option<&TextKey>) -> Self {
        match key {
            Some(key) if crate::text::exists(&key.id) => Self {
                text: text.to_owned(),
                key: Some(key.clone()),
            },
            _ => text.into(),
        }
    }

    /// The label of catalog message `key` (its English text from the catalog).
    pub fn keyed(key: TextKey) -> Self {
        Self {
            text: crate::text::english(&key),
            key: Some(key),
        }
    }
}

/// A label given as English text: keyed when the text is one of the catalog's messages
/// without arguments (`"capture midi"`), else shown as is.
impl From<&str> for UndoLabel {
    fn from(text: &str) -> Self {
        Self {
            key: crate::text::key_of_english(text),
            text: text.to_owned(),
        }
    }
}

impl From<String> for UndoLabel {
    fn from(text: String) -> Self {
        Self {
            key: crate::text::key_of_english(&text),
            text,
        }
    }
}

impl From<&String> for UndoLabel {
    fn from(text: &String) -> Self {
        text.as_str().into()
    }
}

impl From<&UndoLabel> for UndoLabel {
    fn from(label: &UndoLabel) -> Self {
        label.clone()
    }
}

impl std::fmt::Display for UndoLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl PartialEq<&str> for UndoLabel {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

/// The label of catalog message `id` (without arguments): `edit_with(doc::undo("undo.capture.capture"), …)`.
pub fn undo(id: &str) -> UndoLabel {
    UndoLabel::keyed(TextKey::new(id))
}

/// Message `id` without arguments.
fn k(id: &str) -> UndoLabel {
    undo(id)
}

/// Message `id` with a `count` (its plural picks "clip" / "3 clips").
fn kn(id: &str, count: usize) -> UndoLabel {
    UndoLabel::keyed(TextKey::new(id).arg("count", count))
}

/// Message `id` with one argument.
fn ka(id: &str, name: &str, value: impl Into<TextArg>) -> UndoLabel {
    UndoLabel::keyed(TextKey::new(id).arg(name, value))
}

/// `yes` when `on`, else `no`.
fn pick(on: bool, yes: &str, no: &str) -> UndoLabel {
    k(if on { yes } else { no })
}

/// The device's name in the project ("reverb"), or the word "device".
fn device_arg(project: Option<&Project>, id: DeviceId) -> TextArg {
    project
        .and_then(|p| p.devices.get(&id))
        .map(|d| d.name.trim())
        .filter(|n| !n.is_empty())
        .map(TextArg::from)
        .unwrap_or_else(|| TextKey::new("undo.noun.device").into())
}

/// A parameter's name when the device's descriptor is static: a built-in's as its catalog
/// message ("cutoff", in the user's language in the UI), a missing device's in lowercase (a
/// plugin's needs its live instance).
fn param_name(project: Option<&Project>, device: DeviceId, param: ParamId) -> Option<TextArg> {
    let d = project?.devices.get(&device)?;
    let (descriptor, builtin) = match &d.kind {
        DeviceKind::Builtin { device } => (super::builtin_descriptor(device), true),
        DeviceKind::Missing { missing } => (super::missing_descriptor(missing), false),
        DeviceKind::Plugin { .. } => return None,
    };
    let info = descriptor.params.into_iter().find(|i| i.id == param)?;
    let name = info.name.trim();
    if name.is_empty() {
        None
    } else if builtin {
        Some(crate::text::engine_arg(name))
    } else {
        Some(name.to_lowercase().into())
    }
}

/// The `kind` of `undo.track.create`.
fn track_kind(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "audio",
        TrackKind::Midi => "midi",
        TrackKind::Group => "group",
        TrackKind::Return => "return",
        TrackKind::Master => "master",
        TrackKind::Vca => "vca",
    }
}

/// The `kind` of `undo.modulation.add`.
fn modulator_kind(kind: ModulatorKind) -> &'static str {
    match kind {
        ModulatorKind::Lfo => "lfo",
        ModulatorKind::Envelope => "envelope",
        ModulatorKind::EnvelopeFollower => "follower",
        ModulatorKind::Steps => "steps",
        ModulatorKind::Random => "random",
        ModulatorKind::Keytrack => "keytrack",
        ModulatorKind::Velocity => "velocity",
    }
}

/// What inserting `spec` adds: a built-in's name (its catalog message), or the word "plugin".
fn spec_arg(spec: &DeviceSpec) -> TextArg {
    match spec {
        DeviceSpec::Builtin { device } => {
            crate::text::engine_arg(&super::builtin_descriptor(device).name)
        }
        DeviceSpec::Plugin { .. } => TextKey::new("undo.noun.plugin").into(),
    }
}

/// `undo.device.set_param` ("set cutoff"), or `fallback` ("set reverb parameter") when the
/// parameter's name isn't known here.
fn param_label(
    project: Option<&Project>,
    device: DeviceId,
    param: ParamId,
    named: &str,
    fallback: &str,
) -> UndoLabel {
    match param_name(project, device, param) {
        Some(p) => ka(named, "param", p),
        None => ka(fallback, "device", device_arg(project, device)),
    }
}

/// The undo label of `command` (`project`: the open project, to name devices and
/// parameters; `None` gives the generic label).
pub fn label_of(command: &Command, project: Option<&Project>) -> UndoLabel {
    match command {
        Command::Edit(c) => match c {
            E::Undo => k("undo.edit.undo"),
            E::Redo => k("undo.edit.redo"),
            E::EndGesture { .. } => k("undo.edit.end_gesture"),
            E::CancelGesture { .. } => k("undo.edit.cancel_gesture"),
            E::Batch { label, key, .. } => UndoLabel::batch(label, key.as_ref()),
        },
        Command::Track(c) => match c {
            T::Create { kind, .. } => ka("undo.track.create", "kind", track_kind(*kind)),
            T::Delete { .. } => k("undo.track.delete"),
            T::Duplicate { .. } => k("undo.track.duplicate"),
            T::Rename { .. } => k("undo.track.rename"),
            T::SetColor { .. } => k("undo.track.set_color"),
            T::Move { .. } => k("undo.track.move"),
            T::SetScale { .. } => k("undo.track.set_scale"),
            T::GroupSelected { ids, .. } => kn("undo.track.group", ids.len()),
            T::Ungroup { .. } => k("undo.track.ungroup"),
            T::SetVca { vca, .. } => pick(
                vca.is_some(),
                "undo.track.assign_vca",
                "undo.track.unassign_vca",
            ),
        },
        Command::Clip(c) => match c {
            C::CreateMidi { .. } => k("undo.clip.create_midi"),
            C::CreateAudio { .. } => k("undo.clip.create_audio"),
            C::Delete { ids } => kn("undo.clip.delete", ids.len()),
            C::Move { moves } => kn("undo.clip.move", moves.len()),
            C::SetBounds { .. } => k("undo.clip.resize"),
            C::Split { .. } => k("undo.clip.split"),
            C::Duplicate { .. } => k("undo.clip.duplicate"),
            C::Rename { .. } => k("undo.clip.rename"),
            C::SetColor { .. } => k("undo.clip.set_color"),
            C::SetMuted { ids, muted } => kn(
                if *muted {
                    "undo.clip.mute"
                } else {
                    "undo.clip.unmute"
                },
                ids.len(),
            ),
            C::SetLoop { .. } => k("undo.clip.set_loop"),
            C::SetGain { .. } => k("undo.clip.set_gain"),
            C::SetTranspose { .. } => k("undo.clip.transpose"),
            C::Stretch { .. } => k("undo.clip.stretch"),
            C::SetFades { .. } => k("undo.clip.set_fades"),
            C::SetFadeCurves { .. } => k("undo.clip.set_fade_curves"),
            C::SetReversed { reversed, .. } => {
                pick(*reversed, "undo.clip.reverse", "undo.clip.unreverse")
            }
            C::Crossfade { .. } => k("undo.clip.crossfade"),
            C::SetCrossfade { .. } => k("undo.clip.set_crossfade"),
        },
        Command::Note(c) => match c {
            N::Add { notes, .. } => kn("undo.note.add", notes.len()),
            N::Remove { ids } => kn("undo.note.delete", ids.len()),
            N::Edit { edits } => kn("undo.note.edit", edits.len()),
            N::Quantize { .. } => k("undo.note.quantize"),
            N::Reverse { .. } => k("undo.note.reverse"),
            N::Duplicate { copies, .. } => kn("undo.note.duplicate", copies.len()),
        },
        Command::Automation(c) => match c {
            Au::CreateLane { .. } => k("undo.automation.create_lane"),
            Au::DeleteLane { .. } => k("undo.automation.delete_lane"),
            Au::SetLaneEnabled { enabled, .. } => pick(
                *enabled,
                "undo.automation.enable",
                "undo.automation.disable",
            ),
            Au::AddPoints { points, .. } => kn("undo.automation.add_points", points.len()),
            Au::RemovePoints { ids } => kn("undo.automation.delete_points", ids.len()),
            Au::EditPoints { edits } => kn("undo.automation.edit_points", edits.len()),
            Au::ClearRange { .. } => k("undo.automation.clear"),
        },
        Command::Mixer(c) => match c {
            Mx::SetVolume { .. } => k("undo.mixer.set_volume"),
            Mx::SetPan { .. } => k("undo.mixer.set_pan"),
            Mx::SetMute { mute, .. } => pick(*mute, "undo.mixer.mute", "undo.mixer.unmute"),
            Mx::SetSolo { solo, .. } => pick(*solo, "undo.mixer.solo", "undo.mixer.unsolo"),
            Mx::SetOutput { .. } => k("undo.mixer.set_output"),
            Mx::CreateSend { .. } => k("undo.mixer.create_send"),
            Mx::SetSendLevel { .. } => k("undo.mixer.set_send_level"),
            Mx::SetSendPreFader { pre_fader, .. } => pick(
                *pre_fader,
                "undo.mixer.send_pre_fader",
                "undo.mixer.send_post_fader",
            ),
            Mx::DeleteSend { .. } => k("undo.mixer.delete_send"),
        },
        Command::Device(c) => match c {
            D::Insert { device, .. } => ka("undo.device.add", "device", spec_arg(device)),
            D::Remove { id } => ka("undo.device.remove", "device", device_arg(project, *id)),
            D::Move { id, .. } => ka("undo.device.move", "device", device_arg(project, *id)),
            D::Duplicate { id, .. } => {
                ka("undo.device.duplicate", "device", device_arg(project, *id))
            }
            D::Rename { .. } => k("undo.device.rename"),
            D::SetEnabled { id, enabled } => ka(
                if *enabled {
                    "undo.device.enable"
                } else {
                    "undo.device.bypass"
                },
                "device",
                device_arg(project, *id),
            ),
            D::SetParam { device, param, .. } => param_label(
                project,
                *device,
                *param,
                "undo.device.set_param",
                "undo.device.set_device_param",
            ),
            D::ResetParam { device, param } => param_label(
                project,
                *device,
                *param,
                "undo.device.reset_param",
                "undo.device.reset_device_param",
            ),
            D::SetSample { media, .. } => pick(
                media.is_some(),
                "undo.device.load_sample",
                "undo.device.clear_sample",
            ),
            D::ListBuiltin => k("undo.device.list"),
            D::GetDescriptor { .. } => k("undo.device.info"),
            D::SetSidechain { source, .. } => pick(
                source.is_some(),
                "undo.device.set_sidechain",
                "undo.device.remove_sidechain",
            ),
            D::SetMidiSidechain { source, .. } => pick(
                source.is_some(),
                "undo.device.set_midi_sidechain",
                "undo.device.remove_midi_sidechain",
            ),
            D::SetZones { .. } => k("undo.device.set_zones"),
            D::SetLfoCurve { .. } => k("undo.device.set_lfo_curve"),
            D::SetShaperCurve { .. } => k("undo.device.set_shaper_curve"),
            D::RenameMacro { .. } => k("undo.device.rename_macro"),
            D::SetMacroCount { .. } => k("undo.device.set_macro_count"),
            D::SetIr { ir, .. } => {
                pick(ir.is_some(), "undo.device.load_ir", "undo.device.clear_ir")
            }
            D::ListFactoryIrs => k("undo.device.list_irs"),
        },
        Command::Transport(c) => match c {
            Tr::Play => k("undo.transport.play"),
            Tr::Stop => k("undo.transport.stop"),
            Tr::TogglePlay => k("undo.transport.toggle_play"),
            Tr::Locate { .. } => k("undo.transport.locate"),
            Tr::SetLoopEnabled { enabled } => pick(
                *enabled,
                "undo.transport.loop_on",
                "undo.transport.loop_off",
            ),
            Tr::SetLoopRegion { .. } => k("undo.transport.set_loop"),
            Tr::SetTempo { .. } => k("undo.transport.set_tempo"),
            Tr::SetTimeSignature { .. } => k("undo.transport.set_time_signature"),
            Tr::SetMetronome { enabled } => pick(
                *enabled,
                "undo.transport.metronome_on",
                "undo.transport.metronome_off",
            ),
            Tr::TapTempo => k("undo.transport.tap_tempo"),
        },
        Command::Recording(c) => match c {
            Rec::Arm { armed, .. } => pick(*armed, "undo.recording.arm", "undo.recording.disarm"),
            Rec::SetMonitor { .. } => k("undo.recording.set_monitor"),
            Rec::SetInput { .. } => k("undo.recording.set_input"),
            Rec::SetRecording { enabled } => {
                pick(*enabled, "undo.recording.record", "undo.recording.stop")
            }
            Rec::SetPunch { enabled } => pick(
                *enabled,
                "undo.recording.punch_on",
                "undo.recording.punch_off",
            ),
            Rec::SetCountIn { .. } => k("undo.recording.set_count_in"),
            Rec::ListInputs => k("undo.recording.list_inputs"),
            Rec::LiveMidi { .. } => k("undo.recording.live_midi"),
            Rec::PreviewNote { .. } => k("undo.recording.preview_note"),
        },
        Command::Warp(c) => match c {
            W::SetWarp { .. } => k("undo.warp.set"),
            W::AddMarker { .. } => k("undo.warp.add_marker"),
            W::MoveMarker { .. } => k("undo.warp.move_marker"),
            W::RemoveMarker { .. } => k("undo.warp.delete_marker"),
            W::DetectTempo { .. } => k("undo.warp.detect_tempo"),
        },
        Command::Tempo(c) => match c {
            Te::AddTempoPoint { .. } => k("undo.tempo.add_point"),
            Te::EditTempoPoint { .. } => k("undo.tempo.edit_point"),
            Te::RemoveTempoPoints { ids } => kn("undo.tempo.delete_points", ids.len()),
            Te::AddTimeSignature { .. } => k("undo.tempo.add_signature"),
            Te::EditTimeSignature { .. } => k("undo.tempo.edit_signature"),
            Te::RemoveTimeSignatures { ids } => kn("undo.tempo.delete_signatures", ids.len()),
            Te::SetMetronomeSettings { .. } => k("undo.tempo.set_metronome"),
        },
        Command::Marker(c) => match c {
            Mk::Add { .. } => k("undo.marker.add"),
            Mk::Move { .. } => k("undo.marker.move"),
            Mk::Rename { .. } => k("undo.marker.rename"),
            Mk::SetColor { .. } => k("undo.marker.set_color"),
            Mk::Remove { ids } => kn("undo.marker.delete", ids.len()),
        },
        Command::Groove(c) => match c {
            GrooveCommand::Humanize { .. } => k("undo.groove.humanize"),
            GrooveCommand::SetSwing { .. } => k("undo.groove.set_swing"),
        },
        Command::DrumRack(c) => match c {
            Dr::AddPad { .. } => k("undo.drum_rack.add_pad"),
            Dr::RemovePad { .. } => k("undo.drum_rack.delete_pad"),
            Dr::SetPadNote { .. } => k("undo.drum_rack.set_pad_note"),
            Dr::RenamePad { .. } => k("undo.drum_rack.rename_pad"),
            Dr::SetPadColor { .. } => k("undo.drum_rack.set_pad_color"),
            Dr::SetChokeGroup { .. } => k("undo.drum_rack.set_choke_group"),
            Dr::SetPadVolume { .. } => k("undo.drum_rack.set_pad_volume"),
            Dr::SetPadPan { .. } => k("undo.drum_rack.set_pad_pan"),
            Dr::SetPadMute { mute, .. } => pick(
                *mute,
                "undo.drum_rack.mute_pad",
                "undo.drum_rack.unmute_pad",
            ),
            Dr::SetPadSolo { solo, .. } => pick(
                *solo,
                "undo.drum_rack.solo_pad",
                "undo.drum_rack.unsolo_pad",
            ),
            Dr::InsertDevice { device, .. } => ka("undo.device.add", "device", spec_arg(device)),
            Dr::MoveDevice { id, .. } => ka("undo.device.move", "device", device_arg(project, *id)),
            Dr::AddSamplePad { .. } => k("undo.drum_rack.add_sample_pad"),
        },
        Command::Slice(c) => match c {
            Sl::SetEnabled { enabled, .. } => {
                pick(*enabled, "undo.slice.enable", "undo.slice.disable")
            }
            Sl::SetBaseNote { .. } => k("undo.slice.set_base_note"),
            Sl::Add { positions, .. } => kn("undo.slice.add", positions.len()),
            Sl::Move { .. } => k("undo.slice.move"),
            Sl::Remove { indices, .. } => kn("undo.slice.delete", indices.len()),
            Sl::Auto { .. } => k("undo.slice.auto"),
            Sl::ToDrumRack { .. } => k("undo.slice.to_drum_rack"),
            Sl::SampleToPads { .. } => k("undo.slice.to_pads"),
        },
        Command::MidiMap(c) => match c {
            Mm::Learn { target } => pick(
                target.is_some(),
                "undo.midi_map.learn",
                "undo.midi_map.stop_learn",
            ),
            Mm::Map { .. } => k("undo.midi_map.add"),
            Mm::Edit { .. } => k("undo.midi_map.edit"),
            Mm::Unmap { ids } => kn("undo.midi_map.delete", ids.len()),
            Mm::List => k("undo.midi_map.list"),
        },
        Command::Take(c) => match c {
            Tk::CreateLane { .. } => k("undo.take.create_lane"),
            Tk::RemoveLane { .. } => k("undo.take.delete_lane"),
            Tk::RenameLane { .. } => k("undo.take.rename_lane"),
            Tk::SetLaneColor { .. } => k("undo.take.set_lane_color"),
            Tk::MoveLane { .. } => k("undo.take.move_lane"),
            Tk::MoveToLane { clips, .. } => kn("undo.take.move", clips.len()),
            Tk::SetComp { .. } => k("undo.take.comp"),
            Tk::ClearComp { .. } => k("undo.take.clear_comp"),
            Tk::SetCrossfade { .. } => k("undo.take.set_crossfade"),
            Tk::Flatten { .. } => k("undo.take.flatten"),
            Tk::Audition { .. } => k("undo.take.audition"),
        },
        Command::Rack(c) => match c {
            R::AddChain { .. } => k("undo.rack.add_chain"),
            R::RemoveChain { .. } => k("undo.rack.delete_chain"),
            R::RenameChain { .. } => k("undo.rack.rename_chain"),
            R::SetChainColor { .. } => k("undo.rack.set_chain_color"),
            R::MoveChain { .. } => k("undo.rack.move_chain"),
            R::CopyChain { cut, .. } => pick(*cut, "undo.rack.move_chain", "undo.rack.copy_chain"),
            R::SetChainMix { .. } => k("undo.rack.set_chain_mix"),
            R::SetChainZones { .. } => k("undo.rack.set_chain_zones"),
            R::InsertDevice { device, .. } => ka("undo.device.add", "device", spec_arg(device)),
            R::MoveDevice { id, .. } => ka("undo.device.move", "device", device_arg(project, *id)),
            R::Group { devices, .. } => kn("undo.rack.group", devices.len()),
            R::Ungroup { rack } => ka("undo.rack.ungroup", "device", device_arg(project, *rack)),
        },
        Command::Modulation(c) => match c {
            Mo::AddModulator { kind, .. } => {
                ka("undo.modulation.add", "kind", modulator_kind(*kind))
            }
            Mo::RemoveModulator { .. } => k("undo.modulation.delete"),
            Mo::RenameModulator { .. } => k("undo.modulation.rename"),
            Mo::SetModulatorParam { .. } => k("undo.modulation.set"),
            Mo::ResetModulatorParam { .. } => k("undo.modulation.reset"),
            Mo::Map { device, param, .. } => match param_name(project, *device, *param) {
                Some(p) => ka("undo.modulation.map_param", "param", p),
                None => k("undo.modulation.map"),
            },
            Mo::SetDepth { .. } => k("undo.modulation.set_depth"),
            Mo::Unmap { .. } => k("undo.modulation.unmap"),
            Mo::SetSidechain { source, .. } => pick(
                source.is_some(),
                "undo.modulation.set_sidechain",
                "undo.modulation.remove_sidechain",
            ),
            Mo::ListModulatorKinds => k("undo.modulation.list"),
        },
        Command::Expression(c) => match c {
            Ex::CreateLane { .. } => k("undo.expression.create_lane"),
            Ex::RemoveLane { .. } => k("undo.expression.delete_lane"),
            Ex::SetPoints { .. } | Ex::ReplaceRange { .. } => k("undo.expression.draw"),
            Ex::SetNoteExpression { .. } => k("undo.expression.set_note"),
            Ex::ClearNoteExpressions { notes, .. } => {
                kn("undo.expression.clear_notes", notes.len())
            }
            Ex::SetTrackMpe { mpe, .. } => pick(
                mpe.is_some(),
                "undo.expression.set_mpe",
                "undo.expression.mpe_off",
            ),
        },
        Command::External(c) => match c {
            ExternalCommand::SetRouting { .. } => k("undo.external.set_routing"),
            ExternalCommand::ListPorts => k("undo.external.list_ports"),
            ExternalCommand::MeasureLatency { .. } => k("undo.external.measure_latency"),
        },
        Command::Template(c) => match c {
            Tpl::List { .. } => k("undo.template.list"),
            Tpl::SaveProject { .. } => k("undo.template.save_project"),
            Tpl::SaveTracks { tracks, .. } => kn("undo.template.save_tracks", tracks.len()),
            Tpl::Insert { .. } => k("undo.template.insert"),
            Tpl::NewProject { .. } => k("undo.template.new_project"),
            Tpl::Rename { .. } => k("undo.template.rename"),
            Tpl::Delete { .. } => k("undo.template.delete"),
            Tpl::SetDefault { .. } => k("undo.template.set_default"),
        },
        Command::Project(c) => match c {
            P::List => k("undo.project.list"),
            P::Create { .. } => k("undo.project.create"),
            P::Open { .. } | P::OpenSafe { .. } | P::OpenMirrored { .. } => k("undo.project.open"),
            P::LoadPlugins => k("undo.project.load_plugins"),
            P::Save { .. } => k("undo.project.save"),
            P::SaveAs { .. } => k("undo.project.save_as"),
            P::Duplicate { .. } => k("undo.project.duplicate"),
            P::Rename { .. } => k("undo.project.rename"),
            P::Delete { .. } | P::Discard { .. } | P::EmptyBin => k("undo.project.delete"),
            P::MoveToBin { .. } => k("undo.project.move_to_bin"),
            P::RestoreFromBin { .. } => k("undo.project.restore_from_bin"),
            P::ListBin => k("undo.project.list"),
            P::SetArchived { archived, .. } => {
                pick(*archived, "undo.project.archive", "undo.project.unarchive")
            }
            P::Get => k("undo.project.get"),
            P::SetScale { .. } => k("undo.project.set_scale"),
            P::ExportBundle { .. } | P::ExportTo { .. } => k("undo.project.export"),
            P::ImportBundle { .. } | P::Import { .. } => k("undo.project.import"),
            P::CancelImport { .. } => k("undo.project.cancel_import"),
        },
        Command::Plugin(c) => match c {
            Pl::Rescan { .. } => k("undo.plugin.rescan"),
            Pl::List => k("undo.plugin.list"),
            Pl::OpenEditor { .. } => k("undo.plugin.open_editor"),
            Pl::CloseEditor { .. } => k("undo.plugin.close_editor"),
            Pl::SetSandboxed { sandboxed, .. } => {
                pick(*sandboxed, "undo.plugin.sandbox", "undo.plugin.unsandbox")
            }
            Pl::SetAlwaysProcess { always_process, .. } => pick(
                *always_process,
                "undo.plugin.always_process",
                "undo.plugin.sleep_on_silence",
            ),
            Pl::Reload { .. } => k("undo.plugin.reload"),
            Pl::ListFolders => k("undo.plugin.list_folders"),
            Pl::AddFolder { .. } => k("undo.plugin.add_folder"),
            Pl::RemoveFolder { .. } => k("undo.plugin.remove_folder"),
            Pl::SetIncludeDefaults { .. } => k("undo.plugin.set_default_folders"),
            Pl::ListSandboxDefaults => k("undo.plugin.list_sandbox_defaults"),
            Pl::SetSandboxDefault { .. } => k("undo.plugin.set_sandbox_default"),
            Pl::SetEditorKeyCapture { .. } => k("undo.plugin.set_keys"),
            Pl::ListScanFailures => k("undo.plugin.list_failures"),
            Pl::RetryScan { .. } => k("undo.plugin.retry_scan"),
            Pl::ParamTexts { .. } => k("undo.plugin.param_texts"),
        },
        Command::Media(c) => match c {
            Me::Import { .. } => k("undo.media.import"),
            Me::GetPeaks { .. } => k("undo.media.peaks"),
            Me::ListLocations | Me::ListDirectory { .. } => k("undo.media.browse"),
            Me::Preview { .. } => k("undo.media.preview"),
            Me::StopPreview => k("undo.media.stop_preview"),
            Me::BeginUpload { .. } | Me::UploadChunk { .. } => k("undo.media.upload"),
            Me::CancelUpload { .. } => k("undo.media.cancel_upload"),
            Me::ReadMidiFile { .. } => k("undo.media.read_midi"),
            Me::WriteMidiFile { .. } => k("undo.media.export_midi"),
            Me::Reveal { .. } => k("undo.media.reveal"),
        },
        Command::Freeze(c) => match c {
            F::Freeze { .. } => k("undo.freeze.freeze"),
            F::Unfreeze { .. } => k("undo.freeze.unfreeze"),
            F::Flatten { .. } => k("undo.freeze.flatten"),
            F::Bounce { .. } | F::BounceRange { .. } => k("undo.freeze.bounce"),
            F::Consolidate { tracks, .. } => kn("undo.freeze.consolidate_tracks", tracks.len()),
            F::ConvertToSampler { .. } => k("undo.freeze.convert_sampler"),
            F::Cancel { .. } => k("undo.freeze.cancel"),
        },
        Command::TimeEdit(c) => match c {
            Ti::Split { .. } => k("undo.time.split"),
            Ti::Copy { .. } => k("undo.time.copy"),
            Ti::Cut { .. } => k("undo.time.cut"),
            Ti::Paste { insert, .. } => pick(*insert, "undo.time.paste_insert", "undo.time.paste"),
            Ti::DeleteTime { .. } => k("undo.time.delete"),
            Ti::InsertSilence { .. } => k("undo.time.insert_silence"),
            Ti::DuplicateTime { .. } => k("undo.time.duplicate"),
        },
        Command::Preset(c) => match c {
            Pr::List { .. } => k("undo.preset.list"),
            Pr::Load { .. } => k("undo.preset.load"),
            Pr::Save { .. } => k("undo.preset.save"),
            Pr::Rename { .. } => k("undo.preset.rename"),
            Pr::Delete { .. } => k("undo.preset.delete"),
            Pr::SetMeta { .. } => k("undo.preset.edit_tags"),
        },
        Command::MediaRef(c) => match c {
            Mr::ListMissing => k("undo.media_ref.list_missing"),
            Mr::Search { .. } => k("undo.media_ref.search"),
            Mr::Relink { .. } => k("undo.media_ref.relink"),
            Mr::CollectAll => k("undo.media_ref.collect_all"),
            Mr::Remove { media } => kn("undo.media_ref.remove", media.len()),
        },
        Command::Capture(c) => match c {
            CaptureCommand::Capture { .. } => k("undo.capture.capture"),
            CaptureCommand::Clear => k("undo.capture.clear"),
            CaptureCommand::Status => k("undo.capture.status"),
        },
        Command::AudioToMidi(c) => match c {
            AudioToMidiCommand::Start { .. } => k("undo.audio_to_midi.start"),
            AudioToMidiCommand::Cancel { .. } => k("undo.audio_to_midi.cancel"),
        },
        Command::Resolve(c) => match c {
            ResolveCommand::Propose { .. } => k("undo.resolve.propose"),
            ResolveCommand::SearchMedia { .. } => k("undo.media_ref.search"),
            ResolveCommand::CancelSearch => k("undo.resolve.cancel_search"),
            ResolveCommand::Apply { .. } => k("undo.resolve.apply"),
            ResolveCommand::Replace { .. } => k("undo.resolve.replace"),
        },
        Command::Agent(c) => match c {
            AgentCommand::ListTools => k("undo.agent.list_tools"),
            AgentCommand::CallTool { .. } => k("undo.agent.call_tool"),
            AgentCommand::SetSelection { .. } => k("undo.agent.set_selection"),
        },
        // Never document commands, never an undo step: one label per domain.
        Command::Engine(_) => k("undo.domain.engine"),
        Command::Export(_) => k("undo.domain.export"),
        Command::Browser(_) => k("undo.domain.browser"),
        Command::Analysis(_) => k("undo.domain.analysis"),
        Command::History(_) => k("undo.domain.history"),
        Command::Version(_) => k("undo.domain.versions"),
        Command::Keymap(_) => k("undo.domain.keymap"),
        Command::Settings(_) => k("undo.domain.settings"),
        Command::Samples(_) => k("undo.domain.samples"),
        // An extension's command: its own label (`set_ext_labels`), else a generic one.
        Command::Ext(c) => match EXT_LABELS.get().and_then(|f| f(c, project)) {
            Some(key) => UndoLabel::keyed(key),
            None => k("undo.domain.ext"),
        },
    }
}

/// The undo labels of an extension's commands: a catalog message (`None`: the generic
/// label). The catalog is the app's merged with the extension's (docs/LOCALIZATION.md).
pub type ExtLabels = fn(&ExtCommand, Option<&Project>) -> Option<TextKey>;

static EXT_LABELS: std::sync::OnceLock<ExtLabels> = std::sync::OnceLock::new();

/// Extension point: the undo labels of an extension's commands (set once, at start).
pub fn set_ext_labels(labels: ExtLabels) {
    let _ = EXT_LABELS.set(labels);
}

#[cfg(test)]
mod tests;
