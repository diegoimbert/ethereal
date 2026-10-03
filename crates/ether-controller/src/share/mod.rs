//! Sharing (base-115 contract; design: docs/SHARING.md): the P2P host hub, invite links,
//! joining, offline copies. Stubs until the nodes land:
//!
//! - `p2p-transport` (`ether-collab::share::{hub, native, web}`): the hub (the existing
//!   `Relay` state machine, in process), the data-channel `PeerLink`s, the signaling client.
//! - `share-engine` (this module): `ShareCommand` → hub/session lifecycle, `share.json`,
//!   identity, roles, `ShareEvent::State`. It drives the existing collab session
//!   (`collab/mod.rs`) through its `Connector` seam: the host's own site joins the hub over
//!   a loopback link, a joiner's site over its data channel (docs/SHARING.md §2.3).
//!
//! Until then every command but `Get` replies `Unsupported`, and `Get` reports `Off`
//! (`tests/share_prewire.rs`; each node removes only its own assertions there).

use ether_core::protocol::share::{ShareCommand, ShareEvent, ShareState};
use ether_core::protocol::{Event, ReplyValue};

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn share_command(
        &mut self,
        c: &ShareCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            ShareCommand::Get => {
                event(
                    out,
                    Event::Share {
                        event: ShareEvent::State {
                            state: ShareState::Off,
                        },
                    },
                );
                Ok(ReplyValue::Unit)
            }
            // share-engine (identity, servers, host lifecycle, links, members, joiner flow).
            ShareCommand::SetIdentity { .. }
            | ShareCommand::SetServers { .. }
            | ShareCommand::Start
            | ShareCommand::Stop
            | ShareCommand::ResetLink { .. }
            | ShareCommand::RemoveParticipant { .. }
            | ShareCommand::SetParticipantRole { .. }
            | ShareCommand::OpenInvite { .. }
            | ShareCommand::AcceptInvite
            | ShareCommand::Leave
            | ShareCommand::Reconnect { .. }
            | ShareCommand::Detach { .. } => Err(unsupported(
                "sharing is not implemented yet (docs/SHARING.md)",
            )),
            // p2p-transport (web UI endpoint).
            ShareCommand::PeerSignal { .. } => Err(unsupported(
                "the UI peer endpoint is not implemented yet (docs/SHARING.md)",
            )),
        }
    }
}
