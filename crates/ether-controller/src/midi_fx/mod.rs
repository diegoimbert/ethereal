//! MIDI effects: chain ordering and scale data (v0.2, owned by the `midi-fx` node;
//! CONTRACTS.md §12.4.4).
//!
//! - [`check_chain_order`]: called by `doc/devices.rs` after `Device::{Insert, Move}` (and by
//!   rack chain inserts): MIDI effects (`DeviceCategory::NoteEffect`) must precede the first
//!   instrument of a chain; `InvalidArgument` otherwise. Placeholder: accepts everything.
//! - Scale Quantize: push the resolved `MusicalScale` (track scale, else project scale) to
//!   the node with `EngineBridge::update_builtin`-style node data when the device is created
//!   and whenever the track/project scale changes (add the hook in this module; the node
//!   reads it in `Node::set_data`).

use ether_core::protocol::model::{Project, TrackId};

use crate::tx::CmdResult;

/// Check the MIDI-effect ordering rule on `track`'s chain.
pub(crate) fn check_chain_order(p: &Project, track: TrackId) -> CmdResult<()> {
    let _ = (p, track);
    Ok(())
}
