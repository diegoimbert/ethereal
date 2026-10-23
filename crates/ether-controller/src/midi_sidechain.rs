//! MIDI sidechains (CONTRACTS.md §11.10 "MIDI sidechain"): an audio effect's note input fed
//! from a MIDI track (`ether_model::MidiSidechain`).
//!
//! `DeviceCommand::SetMidiSidechain` is a document command ([`set_midi_sidechain`], from
//! `doc/devices.rs`): one `DeviceChange::MidiSidechain` op, so one undo step. Validation (on
//! top of the model's, which rejects non-MIDI sources and routing cycles):
//! - the device must be an audio effect with a note input (`DeviceDescriptor::midi_input`:
//!   built-ins that use notes, plugins whose effect declares a note port / event bus).
//!
//! [`desc`] compiles `TrackDesc::midi_sidechains`; the engine side (ordering, PDC, the
//! note routing) is `ether-core/src/midi_sidechain.rs`. Deleting the source track cuts the
//! MIDI sidechain (`doc/mod.rs`). Clearing (`None`) is always allowed; setting the current
//! value is a no-op.

use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::{
    Device, DeviceChange, DeviceId, MidiSidechain, MidiTap, Project, TrackId, TrackKind,
};
use ether_core::{MidiPoint, MidiSidechainDesc};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid};

pub fn set_midi_sidechain(
    ctx: &mut DocCtx,
    device: DeviceId,
    source: Option<MidiSidechain>,
) -> CmdResult<()> {
    let d = ctx.device(device)?;
    if d.midi_sidechain == source {
        return Ok(());
    }
    if let Some(m) = source {
        if ctx.track(m.track)?.kind != TrackKind::Midi {
            return Err(invalid("a MIDI sidechain source must be a MIDI track"));
        }
        let note_input = ctx
            .host
            .descriptor(d.id, &d.kind)
            .is_some_and(|desc| has_note_input(&desc));
        if !note_input {
            return Err(invalid(format!("device {} has no note input", d.id)));
        }
    }
    ctx.set_device(d.id, DeviceChange::MidiSidechain(source))
}

/// An audio effect with a note input (the devices a MIDI sidechain is for).
pub fn has_note_input(desc: &ether_core::protocol::devices::DeviceDescriptor) -> bool {
    desc.category == DeviceCategory::AudioEffect && desc.midi_input
}

/// Clears device `d`'s MIDI sidechain.
pub fn cut(ctx: &mut DocCtx, d: &Device) -> CmdResult<()> {
    if d.midi_sidechain.is_some() {
        ctx.set_device(d.id, DeviceChange::MidiSidechain(None))?;
    }
    Ok(())
}

/// `TrackDesc::midi_sidechains` of `track`: every MIDI-sidechained device on it (any depth)
/// with a node, from a source that plays notes (frozen tracks play none).
pub fn desc(
    p: &Project,
    track: TrackId,
    ctx: &crate::compile::CompileContext,
) -> Vec<MidiSidechainDesc> {
    let mut out: Vec<MidiSidechainDesc> = p
        .devices
        .values()
        .filter(|d| d.track == track)
        .filter_map(|d| {
            let m = d.midi_sidechain?;
            let source = p.tracks.get(&m.track)?;
            if source.freeze.is_some() || p.frozen_ancestor(source.id).is_some() {
                return None;
            }
            Some(MidiSidechainDesc {
                node: (ctx.nodes)(d.id)?,
                source: m.track,
                point: engine_point(p, m.track, m.tap, ctx),
            })
        })
        .collect();
    out.sort_by_key(|m| m.node);
    out
}

/// The engine point of `tap` on `source`: its chain's input (pre FX); post FX, the notes
/// reaching its first device that isn't a MIDI effect, else (MIDI effects only) its chain's
/// end. Devices without a node are not in the compiled chain, so they don't count.
fn engine_point(
    p: &Project,
    source: TrackId,
    tap: MidiTap,
    ctx: &crate::compile::CompileContext,
) -> MidiPoint {
    if tap == MidiTap::PreFx {
        return MidiPoint::Input;
    }
    let chain: Vec<(&Device, ether_core::NodeKey)> = p
        .devices_of(source)
        .into_iter()
        .filter_map(|d| Some((d, (ctx.nodes)(d.id)?)))
        .collect();
    if chain.is_empty() {
        return MidiPoint::Input;
    }
    chain
        .iter()
        .find(|(d, _)| {
            (ctx.descriptors)(d).is_none_or(|desc| desc.category != DeviceCategory::NoteEffect)
        })
        .map_or(MidiPoint::End, |&(_, node)| MidiPoint::Before(node))
}
