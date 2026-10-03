//! A joiner's collab transport over its data channel (docs/SHARING.md §2.1, §7.2).
//!
//! The collab session opens transports through a [`Connector`]; a joiner's link to the host
//! only exists after signaling, ICE and the handshake, which the share module drives in its
//! tick. So the session's connector hands out [`SlotTransport`]s of a [`Port`]: a slot is
//! `Connecting` until the share module [`Port::install`]s an authenticated link, then
//! carries `CollabMessage` frames over it. When the link drops, the session's usual backoff
//! asks the connector again, which raises [`Port::wanted`]: the share module dials the host
//! again (member key) and installs the new link. [`Port::end`] makes the slot fatal (sharing
//! ended).

use std::sync::{Arc, Mutex};

use ether_collab::share::BoxPeerLink;
use ether_collab::wire::{WireFrame, decode_binary, decode_text, encode_frame};
use ether_collab::{
    BoxTransport, CollabMessage, CollabTransport, ConnectRequest, Connector, LinkState,
};

#[derive(Default)]
struct Inner {
    /// Bumped by every connector call; only the newest slot is live.
    generation: u64,
    link: Option<BoxPeerLink>,
    /// Installed and not yet handed to a slot (the next connector call keeps it).
    fresh: bool,
    /// The session asked for a link and none is installed.
    wanted: bool,
    /// Sharing ended: every slot is closed for good.
    ended: Option<String>,
    /// Bytes received on the current link (the `Syncing` progress).
    received: u64,
}

#[derive(Clone, Default)]
pub(crate) struct Port {
    inner: Arc<Mutex<Inner>>,
}

impl Port {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("share port")
    }

    /// The collab session's connector.
    pub(crate) fn connector(&self) -> Connector {
        let port = self.clone();
        Box::new(move |_: &ConnectRequest| -> BoxTransport {
            let mut p = port.lock();
            p.generation += 1;
            let keep = p.fresh
                && p.link
                    .as_ref()
                    .is_some_and(|l| l.state() == LinkState::Open);
            if !keep {
                if let Some(mut l) = p.link.take() {
                    l.close();
                }
                p.wanted = p.ended.is_none();
            }
            p.fresh = false;
            Box::new(SlotTransport {
                port: port.clone(),
                generation: p.generation,
            })
        })
    }

    /// An authenticated link (after `Accept`) for the current or next slot.
    pub(crate) fn install(&self, link: BoxPeerLink) {
        let mut p = self.lock();
        if let Some(mut old) = p.link.replace(link) {
            old.close();
        }
        p.fresh = true;
        p.wanted = false;
        p.received = 0;
    }

    /// The session wants a link and none is installed (dial the host).
    pub(crate) fn wanted(&self) -> bool {
        let p = self.lock();
        p.wanted && p.link.is_none() && p.ended.is_none()
    }

    /// The installed link is open.
    pub(crate) fn link_open(&self) -> bool {
        self.lock()
            .link
            .as_ref()
            .is_some_and(|l| l.state() == LinkState::Open)
    }

    /// Bytes received on the current link.
    pub(crate) fn received(&self) -> u64 {
        self.lock().received
    }

    /// Close for good (sharing ended, left): the session's slot reports a fatal close.
    pub(crate) fn end(&self, reason: &str) {
        let mut p = self.lock();
        p.ended = Some(reason.to_string());
        p.wanted = false;
        if let Some(mut l) = p.link.take() {
            l.close();
        }
    }
}

struct SlotTransport {
    port: Port,
    generation: u64,
}

impl SlotTransport {
    fn with_link<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
        let mut p = self.port.lock();
        (p.generation == self.generation && p.ended.is_none()).then(|| f(&mut p))
    }
}

impl CollabTransport for SlotTransport {
    fn send(&mut self, message: &CollabMessage) {
        self.with_link(|p| {
            if let Some(l) = p.link.as_mut() {
                l.send(&encode_frame(message));
            }
        });
    }

    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
        self.with_link(|p| {
            let Some(l) = p.link.as_mut() else { return };
            let mut frames = Vec::new();
            l.poll(&mut frames);
            for f in frames {
                let (m, n) = match f {
                    WireFrame::Text(t) => (decode_text(&t), t.len()),
                    WireFrame::Binary(b) => (decode_binary(&b), b.len()),
                };
                p.received += n as u64;
                match m {
                    Ok(m) => out.push(m),
                    Err(_) => {
                        // A host sending garbage: drop the link (the session reconnects).
                        l.close();
                        return;
                    }
                }
            }
        });
    }

    fn state(&self) -> LinkState {
        let p = self.port.lock();
        if let Some(reason) = &p.ended {
            return LinkState::Closed {
                reason: reason.clone(),
                fatal: true,
            };
        }
        if p.generation != self.generation {
            return LinkState::Closed {
                reason: "replaced".into(),
                fatal: false,
            };
        }
        match &p.link {
            None => LinkState::Connecting,
            Some(l) => l.state(),
        }
    }

    fn close(&mut self) {
        self.with_link(|p| {
            if let Some(mut l) = p.link.take() {
                l.close();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_collab::share::fake::pipe;
    use ether_core::protocol::model::SiteId;

    #[test]
    fn slots_follow_installed_links() {
        let port = Port::default();
        let mut connect = port.connector();
        let req = ConnectRequest {
            server: "share://r".into(),
            session: "share".into(),
            token: None,
            client: String::new(),
        };
        // Installed before the session connects: the first slot keeps it.
        let (a, mut b) = pipe();
        port.install(a);
        let mut slot = connect(&req);
        assert!(!port.wanted());
        assert_eq!(slot.state(), LinkState::Open);
        slot.send(&CollabMessage::Leave { site: SiteId(1) });
        let mut frames = Vec::new();
        b.poll(&mut frames);
        assert_eq!(frames.len(), 1);
        b.send(&encode_frame(&CollabMessage::Leave { site: SiteId(2) }));
        let mut got = Vec::new();
        slot.poll(&mut got);
        assert_eq!(got, [CollabMessage::Leave { site: SiteId(2) }]);
        assert!(port.received() > 0);
        // The link drops: the session reconnects and the port wants a new link.
        b.close();
        assert!(matches!(
            slot.state(),
            LinkState::Closed { fatal: false, .. }
        ));
        let slot2 = connect(&req);
        assert!(port.wanted());
        assert_eq!(slot2.state(), LinkState::Connecting);
        assert!(
            matches!(slot.state(), LinkState::Closed { .. }),
            "superseded"
        );
        let (c, _d) = pipe();
        port.install(c);
        assert_eq!(slot2.state(), LinkState::Open);
        port.end("sharing ended");
        assert!(matches!(
            slot2.state(),
            LinkState::Closed { fatal: true, .. }
        ));
    }
}
