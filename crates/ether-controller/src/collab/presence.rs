//! Presence v2 (`presence-v2` node; docs/COLLAB.md §8): the high-rate arranger pointer.
//! The other presence v2 fields (viewport, activity, following) travel in `PresenceState`
//! through the existing `SetPresence` path and need no controller code.
//!
//! - `SetPointer { Some }` is throttled to [`POINTER_MAX_HZ`]: the **latest** value wins and
//!   a throttled one is sent on the next tick once the interval has passed, so the last
//!   position always goes out. Once the pointer has been still for [`SETTLE_RESEND_MS`], its
//!   last position is sent once more (the relay drops pointers that arrive closer than its
//!   own 40 Hz cap after network jitter; this repairs a dropped final position).
//! - `SetPointer { None }` (a clear) is never throttled and drops a pending position.
//! - Peers' pointers become `CollabEvent::Pointer` (our own are dropped by the dispatcher);
//!   a peer's `Leave` emits a clear for it when its pointer was shown.

use std::collections::BTreeSet;

use ether_core::protocol::collab::{ArrangerPointer, CollabEvent, CollabMessage, POINTER_MAX_HZ};
use ether_core::protocol::model::SiteId;
use ether_core::protocol::{Event, ReplyValue};

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Minimum spacing between two sent positions (rounded up: at most `POINTER_MAX_HZ`).
pub(crate) const POINTER_INTERVAL_MS: u64 = 1000_u64.div_ceil(POINTER_MAX_HZ as u64);
/// A still pointer's last position is re-sent once after this long.
pub(crate) const SETTLE_RESEND_MS: u64 = 250;

/// Pointer state of a session (one field of `collab::Session`).
#[derive(Default)]
pub(crate) struct PointerState {
    /// Latest position not sent yet (throttled or waiting for the link).
    pending: Option<ArrangerPointer>,
    /// Last position sent, while peers may be showing it (cleared by a sent clear).
    shown: Option<ArrangerPointer>,
    /// When the last pointer message (position or clear) went out.
    last_sent_ms: Option<u64>,
    /// `shown` still owes its one settle re-send.
    settle_owed: bool,
    /// Peers whose pointer the UI currently shows (to clear them on `Leave`).
    peers: BTreeSet<SiteId>,
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `CollabCommand::SetPointer`. Outside a session this is a no-op (like `SetPresence`).
    pub(crate) fn collab_set_pointer(
        &mut self,
        pointer: &Option<ArrangerPointer>,
        now: u64,
    ) -> CmdResult<ReplyValue> {
        let Some(s) = self.collab.session.as_mut() else {
            return Ok(ReplyValue::Unit);
        };
        let site = self.collab.site;
        let ready = s.open() && s.joined;
        let p = &mut s.pointer;
        match pointer {
            None => {
                p.pending = None;
                p.settle_owed = false;
                if p.shown.take().is_some()
                    && ready
                    && let Some(site) = site
                {
                    p.last_sent_ms = Some(now);
                    s.send(&CollabMessage::Pointer {
                        site,
                        pointer: None,
                    });
                }
            }
            Some(ptr) => {
                p.pending = Some(ptr.clone());
                self.collab_pointer_tick(now);
            }
        }
        Ok(ReplyValue::Unit)
    }

    /// A peer's `CollabMessage::Pointer` (relay-stamped `site`).
    pub(crate) fn collab_pointer_message(
        &mut self,
        site: SiteId,
        pointer: Option<ArrangerPointer>,
        out: &mut dyn MessageSink,
    ) {
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        if pointer.is_some() {
            s.pointer.peers.insert(site);
        } else {
            s.pointer.peers.remove(&site);
        }
        event(
            out,
            Event::Collab {
                event: CollabEvent::Pointer { site, pointer },
            },
        );
    }

    /// Every collab tick (and after a `SetPointer`): send a throttled position once the
    /// interval has passed, or the settle re-send.
    pub(crate) fn collab_pointer_tick(&mut self, now: u64) {
        let Some(site) = self.collab.site else { return };
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        if !(s.open() && s.joined) {
            return;
        }
        let p = &mut s.pointer;
        let since = p.last_sent_ms.map(|t| now.saturating_sub(t));
        let due = since.is_none_or(|d| d >= POINTER_INTERVAL_MS);
        let msg = if let Some(ptr) = p.pending.take_if(|_| due) {
            p.shown = Some(ptr.clone());
            p.settle_owed = true;
            Some(ptr)
        } else if p.pending.is_none()
            && p.settle_owed
            && since.is_some_and(|d| d >= SETTLE_RESEND_MS)
        {
            p.settle_owed = false;
            p.shown.clone()
        } else {
            None
        };
        if let Some(ptr) = msg {
            p.last_sent_ms = Some(now);
            s.send(&CollabMessage::Pointer {
                site,
                pointer: Some(ptr),
            });
        }
    }

    /// Peer `site` left the session: clear its pointer if the UI shows one.
    pub(crate) fn collab_pointer_peer_left(&mut self, site: SiteId, out: &mut dyn MessageSink) {
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        if s.pointer.peers.remove(&site) {
            event(
                out,
                Event::Collab {
                    event: CollabEvent::Pointer {
                        site,
                        pointer: None,
                    },
                },
            );
        }
    }
}
