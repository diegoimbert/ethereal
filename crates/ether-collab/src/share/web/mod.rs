//! Web [`PeerEndpoint`]: the UI's `RTCPeerConnection`s behind the share `MessagePort`
//! (docs/SHARING.md §6.2). `RTCPeerConnection` does not exist in Workers, so the controller
//! Worker asks the UI thread to create one per pairing and exchanges everything with it on
//! one port, installed at startup ([`install_port`], from `ether-wasm`):
//!
//! | Worker → UI | UI → Worker |
//! |---|---|
//! | `{type:"open", ch, offer, ice, relay}` | `{type:"signal", ch, signal}` (`StreamSignal` JSON) |
//! | `{type:"signal", ch, signal}` | `{type:"open", ch, local, remote}` (DTLS fingerprints) |
//! | `{type:"data", ch, data: ArrayBuffer}` (one data-channel message, transferred) | `{type:"data", ch, data}` |
//! | `{type:"close", ch}` | `{type:"flow", ch, consumed, buffered}`, `{type:"closed", ch, reason}` |
//!
//! `ch` is a channel number chosen here (unique per pairing, so a pairing that is
//! replaced never receives the old one's messages). Data-channel bytes never touch the JSON
//! protocol: each message is a fragment of [`dc`] (≤ 16 KiB), posted as a transferred
//! `ArrayBuffer` (zero-copy). Backpressure: [`PeerLink::buffered`] is the bytes posted and
//! not yet handed to the data channel (`consumed`, reported by the UI) plus its
//! `bufferedAmount`. The UI side is `ui/src/features/share/endpoint/` (same message types).
//!
//! The contract's `ShareEvent::{PeerEndpoint, PeerSignal}` / `ShareCommand::PeerSignal` are
//! not used: the port carries the open/close requests and signals too, so the controller
//! drives this endpoint through [`PeerEndpoint`] exactly like the native one.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

use ether_protocol::collab::{IceServer, StreamSignal};
use ether_protocol::share::PeerId;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, MessagePort};

use super::{BoxPeerLink, PeerEndpoint, PeerLink, PeerOutput, dc};
use crate::LinkState;
use crate::wire::WireFrame;

/// The one share port of this Worker (set by [`install_port`]).
struct Hub {
    port: MessagePort,
    next_ch: u32,
    chans: HashMap<u32, Chan>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

/// One pairing, as the hub routes it.
struct Chan {
    peer: PeerId,
    /// The endpoint's output queue (gone once the endpoint is dropped).
    outputs: Weak<RefCell<VecDeque<PeerOutput>>>,
    /// The data channel is open: its link.
    link: Option<Rc<RefCell<LinkInner>>>,
}

#[derive(Default)]
struct LinkInner {
    inbound: VecDeque<WireFrame>,
    reasm: dc::Reassembler,
    state: Option<LinkState>,
    /// Bytes posted to the UI.
    posted: f64,
    /// Of those, handed to the data channel (UI report).
    consumed: f64,
    /// The data channel's `bufferedAmount` (UI report).
    buffered: f64,
}

impl LinkInner {
    fn close(&mut self, reason: impl Into<String>) {
        if !matches!(self.state, Some(LinkState::Closed { .. })) {
            self.state = Some(LinkState::Closed {
                reason: reason.into(),
                fatal: false,
            });
        }
    }
}

thread_local! {
    static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) };
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ToUi<'a> {
    Open {
        ch: u32,
        offer: bool,
        ice: &'a [IceServer],
        relay: bool,
    },
    Signal {
        ch: u32,
        signal: &'a StreamSignal,
    },
    Close {
        ch: u32,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum FromUi {
    Signal {
        ch: u32,
        signal: StreamSignal,
    },
    Open {
        ch: u32,
        local: String,
        remote: String,
    },
    Flow {
        ch: u32,
        consumed: f64,
        buffered: f64,
    },
    Closed {
        ch: u32,
        reason: String,
    },
}

/// Install the share port (once, at Worker startup; a second call replaces it and drops
/// every pairing of the first).
pub fn install_port(port: MessagePort) {
    let on_message =
        Closure::<dyn FnMut(MessageEvent)>::new(|e: MessageEvent| on_message(&e.data()));
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    let old = HUB.with(|h| {
        h.borrow_mut().replace(Hub {
            port,
            next_ch: 1,
            chans: HashMap::new(),
            _on_message: on_message,
        })
    });
    if let Some(old) = old {
        old.port.set_onmessage(None);
        for c in old.chans.into_values() {
            end(c, "the share port was replaced".into());
        }
    }
}

/// Whether a share port is installed.
pub fn has_port() -> bool {
    HUB.with(|h| h.borrow().is_some())
}

/// The DTLS fingerprint as the handshake uses it (`sha-256 AB:CD:...`): lowercase hash
/// name, uppercase hex.
pub fn normalize_fingerprint(fp: &str) -> String {
    let fp = fp.trim();
    match fp.split_once(char::is_whitespace) {
        Some((alg, hex)) => format!(
            "{} {}",
            alg.to_ascii_lowercase(),
            hex.trim().to_ascii_uppercase()
        ),
        None => fp.to_ascii_uppercase(),
    }
}

fn post_json(port: &MessagePort, message: &ToUi<'_>) {
    let json = serde_json::to_string(message).expect("share port messages serialize");
    if let Ok(v) = js_sys::JSON::parse(&json) {
        let _ = port.post_message(&v);
    }
}

fn post(message: &ToUi<'_>) {
    HUB.with(|h| {
        if let Some(hub) = h.borrow().as_ref() {
            post_json(&hub.port, message);
        }
    });
}

fn post_data(ch: u32, bytes: &[u8]) {
    HUB.with(|h| {
        let h = h.borrow();
        let Some(hub) = h.as_ref() else { return };
        let data = js_sys::Uint8Array::from(bytes).buffer();
        let msg = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&msg, &"type".into(), &"data".into());
        let _ = js_sys::Reflect::set(&msg, &"ch".into(), &JsValue::from(ch));
        let _ = js_sys::Reflect::set(&msg, &"data".into(), &data);
        let _ = hub
            .port
            .post_message_with_transferable(&msg, &js_sys::Array::of1(&data));
    });
}

/// The end of a pairing: `Failed` before the channel opened, the link closed after.
fn end(chan: Chan, reason: String) {
    match chan.link {
        Some(link) => link.borrow_mut().close(reason),
        None => {
            if let Some(out) = chan.outputs.upgrade() {
                out.borrow_mut().push_back(PeerOutput::Failed {
                    peer: chan.peer,
                    reason,
                });
            }
        }
    }
}

fn on_message(data: &JsValue) {
    let ty = js_sys::Reflect::get(data, &"type".into())
        .ok()
        .and_then(|t| t.as_string());
    if ty.as_deref() == Some("data") {
        let ch = js_sys::Reflect::get(data, &"ch".into())
            .ok()
            .and_then(|c| c.as_f64())
            .map(|c| c as u32);
        let bytes = js_sys::Reflect::get(data, &"data".into())
            .ok()
            .and_then(|d| d.dyn_into::<js_sys::ArrayBuffer>().ok())
            .map(|b| js_sys::Uint8Array::new(&b).to_vec());
        if let (Some(ch), Some(bytes)) = (ch, bytes) {
            on_data(ch, &bytes);
        }
        return;
    }
    let message = js_sys::JSON::stringify(data)
        .ok()
        .and_then(|s| s.as_string())
        .and_then(|s| serde_json::from_str::<FromUi>(&s).ok());
    let Some(message) = message else {
        tracing::debug!("malformed message on the share port");
        return;
    };
    HUB.with(|h| {
        let mut h = h.borrow_mut();
        let Some(hub) = h.as_mut() else { return };
        match message {
            FromUi::Signal { ch, signal } => {
                if let Some(c) = hub.chans.get(&ch)
                    && let Some(out) = c.outputs.upgrade()
                {
                    out.borrow_mut().push_back(PeerOutput::Signal {
                        peer: c.peer,
                        signal,
                    });
                }
            }
            FromUi::Open { ch, local, remote } => {
                let Some(c) = hub.chans.get_mut(&ch) else {
                    return;
                };
                if c.link.is_some() {
                    return;
                }
                let Some(out) = c.outputs.upgrade() else {
                    return;
                };
                let inner = Rc::new(RefCell::new(LinkInner {
                    state: Some(LinkState::Open),
                    ..LinkInner::default()
                }));
                c.link = Some(inner.clone());
                out.borrow_mut().push_back(PeerOutput::Connected {
                    peer: c.peer,
                    link: Box::new(WebLink { ch, inner }) as BoxPeerLink,
                    local_fingerprint: normalize_fingerprint(&local),
                    remote_fingerprint: normalize_fingerprint(&remote),
                });
            }
            FromUi::Flow {
                ch,
                consumed,
                buffered,
            } => {
                if let Some(link) = hub.chans.get(&ch).and_then(|c| c.link.as_ref()) {
                    let mut l = link.borrow_mut();
                    l.consumed = consumed;
                    l.buffered = buffered;
                }
            }
            FromUi::Closed { ch, reason } => {
                if let Some(c) = hub.chans.remove(&ch) {
                    end(c, reason);
                }
            }
        }
    });
}

fn on_data(ch: u32, bytes: &[u8]) {
    let link = HUB.with(|h| {
        h.borrow()
            .as_ref()
            .and_then(|hub| hub.chans.get(&ch))
            .and_then(|c| c.link.clone())
    });
    let Some(link) = link else { return };
    let mut l = link.borrow_mut();
    if !matches!(l.state, Some(LinkState::Open)) {
        return;
    }
    match l.reasm.push(bytes) {
        Ok(Some(frame)) => l.inbound.push_back(frame),
        Ok(None) => {}
        Err(e) => {
            l.close(format!("bad data channel message: {e}"));
            drop(l);
            close_ch(ch);
        }
    }
}

/// Forget pairing `ch` and ask the UI to close its peer connection.
fn close_ch(ch: u32) {
    let had = HUB.with(|h| {
        h.borrow_mut()
            .as_mut()
            .is_some_and(|hub| hub.chans.remove(&ch).is_some())
    });
    if had {
        post(&ToUi::Close { ch });
    }
}

/// An open data channel, through the UI (see the module docs).
pub struct WebLink {
    ch: u32,
    inner: Rc<RefCell<LinkInner>>,
}

impl PeerLink for WebLink {
    fn send(&mut self, frame: &WireFrame) {
        if !matches!(self.inner.borrow().state, Some(LinkState::Open)) {
            return;
        }
        let mut n = 0usize;
        for f in dc::fragment(frame) {
            n += f.len();
            post_data(self.ch, &f);
        }
        self.inner.borrow_mut().posted += n as f64;
    }

    fn poll(&mut self, out: &mut Vec<WireFrame>) {
        out.extend(self.inner.borrow_mut().inbound.drain(..));
    }

    fn buffered(&self) -> usize {
        let l = self.inner.borrow();
        if !matches!(l.state, Some(LinkState::Open)) {
            return 0;
        }
        ((l.posted - l.consumed).max(0.0) + l.buffered.max(0.0)) as usize
    }

    fn state(&self) -> LinkState {
        self.inner
            .borrow()
            .state
            .clone()
            .unwrap_or(LinkState::Connecting)
    }

    fn close(&mut self) {
        let open = matches!(self.inner.borrow().state, Some(LinkState::Open));
        if open {
            self.inner.borrow_mut().close("closed");
            close_ch(self.ch);
        }
    }
}

impl Drop for WebLink {
    fn drop(&mut self) {
        self.close();
    }
}

/// The web [`PeerEndpoint`] (see the module docs). Without an installed port every peer
/// fails at once.
pub struct WebPeers {
    outputs: Rc<RefCell<VecDeque<PeerOutput>>>,
    relay_only: bool,
}

impl WebPeers {
    pub fn new() -> Self {
        Self {
            outputs: Rc::new(RefCell::new(VecDeque::new())),
            relay_only: false,
        }
    }

    /// The channels of this endpoint for `peer`.
    fn chans_of(&self, peer: PeerId) -> Vec<u32> {
        HUB.with(|h| {
            h.borrow()
                .as_ref()
                .map(|hub| {
                    hub.chans
                        .iter()
                        .filter(|(_, c)| {
                            c.peer == peer && c.outputs.as_ptr() == Rc::as_ptr(&self.outputs)
                        })
                        .map(|(&ch, _)| ch)
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn close_peer(&mut self, peer: PeerId) {
        for ch in self.chans_of(peer) {
            let chan = HUB.with(|h| {
                h.borrow_mut()
                    .as_mut()
                    .and_then(|hub| hub.chans.remove(&ch))
            });
            if let Some(c) = chan {
                if let Some(link) = &c.link {
                    link.borrow_mut().close("closed");
                }
                post(&ToUi::Close { ch });
            }
        }
    }
}

impl Default for WebPeers {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerEndpoint for WebPeers {
    fn open(&mut self, peer: PeerId, offer: bool, ice_servers: &[IceServer]) {
        self.close_peer(peer);
        let ch = HUB.with(|h| {
            let mut h = h.borrow_mut();
            let hub = h.as_mut()?;
            let ch = hub.next_ch;
            hub.next_ch = hub.next_ch.wrapping_add(1).max(1);
            hub.chans.insert(
                ch,
                Chan {
                    peer,
                    outputs: Rc::downgrade(&self.outputs),
                    link: None,
                },
            );
            Some(ch)
        });
        match ch {
            Some(ch) => post(&ToUi::Open {
                ch,
                offer,
                ice: ice_servers,
                relay: self.relay_only,
            }),
            None => self.outputs.borrow_mut().push_back(PeerOutput::Failed {
                peer,
                reason: "peer-to-peer sharing is not available: no share port".into(),
            }),
        }
    }

    fn signal(&mut self, peer: PeerId, signal: StreamSignal) {
        if let Some(&ch) = self.chans_of(peer).iter().max() {
            post(&ToUi::Signal {
                ch,
                signal: &signal,
            });
        }
    }

    fn poll(&mut self, out: &mut Vec<PeerOutput>) {
        out.extend(self.outputs.borrow_mut().drain(..));
    }

    fn close(&mut self, peer: PeerId) {
        self.close_peer(peer);
    }

    fn set_relay_only(&mut self, relay_only: bool) {
        self.relay_only = relay_only;
    }
}

impl Drop for WebPeers {
    fn drop(&mut self) {
        let mine: Vec<PeerId> = HUB.with(|h| {
            h.borrow()
                .as_ref()
                .map(|hub| {
                    hub.chans
                        .values()
                        .filter(|c| c.outputs.as_ptr() == Rc::as_ptr(&self.outputs))
                        .map(|c| c.peer)
                        .collect()
                })
                .unwrap_or_default()
        });
        for peer in mine {
            self.close_peer(peer);
        }
    }
}
