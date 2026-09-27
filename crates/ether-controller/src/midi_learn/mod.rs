//! MIDI learn / controller mappings (roadmap v2, owned by the `midi-learn` node; see
//! `docs/ROADMAP.md` and `ether_protocol::midi_map`).
//!
//! - Document part (`Map`/`Edit`/`Unmap`): [`apply`], dispatched from `doc::apply`.
//! - Runtime part (`Learn`, `List`): [`EtherController::midi_map_command`].
//! - Input: [`EtherController::midi_learn_tick`] drains `EngineBridge::poll_midi_input`,
//!   completes a pending learn, and turns mapped messages into ordinary edits (one gesture
//!   per control, ended after ~300 ms without messages).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::midi_map::MidiMapCommand;

use crate::doc::DocCtx;
use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};
use crate::tx::{CmdResult, unsupported};

pub(crate) fn apply(ctx: &mut DocCtx, c: &MidiMapCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("MIDI mapping is not implemented yet (midi-learn node)"))
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
        let _ = (c, out);
        Err(unsupported("MIDI learn is not implemented yet (midi-learn node)"))
    }

    /// Called from every tick.
    pub(crate) fn midi_learn_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}
