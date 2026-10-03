//! Web signaling socket: the controller Worker's `WebSocket` (wasm32). Callbacks run on the
//! Worker's event loop between controller calls; they only push into a shared queue that
//! `poll` drains.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ether_protocol::share::{SignalClientMessage, SignalServerMessage};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

use super::{decode, encode, socket_url};
use crate::LinkState;
use crate::share::SignalLink;

#[derive(Default)]
struct Inner {
    state: Option<LinkState>,
    inbound: VecDeque<SignalServerMessage>,
    /// Sent before the socket was open.
    backlog: Vec<String>,
}

pub struct WebSignal {
    ws: Option<WebSocket>,
    inner: Rc<RefCell<Inner>>,
    _on_open: Option<Closure<dyn FnMut(Event)>>,
    _on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
    _on_close: Option<Closure<dyn FnMut(CloseEvent)>>,
}

impl WebSignal {
    pub fn connect(url: &str) -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            state: Some(LinkState::Connecting),
            ..Inner::default()
        }));
        let dead = |inner: Rc<RefCell<Inner>>, reason: String| {
            inner.borrow_mut().state = Some(LinkState::Closed {
                reason,
                fatal: true,
            });
            Self {
                ws: None,
                inner,
                _on_open: None,
                _on_message: None,
                _on_close: None,
            }
        };
        let url = match socket_url(url) {
            Ok(u) => u,
            Err(e) => return dead(inner, e),
        };
        let ws = match WebSocket::new(&url) {
            Ok(ws) => ws,
            Err(_) => return dead(inner, format!("cannot open {url}")),
        };
        let on_open = {
            let (ws, inner) = (ws.clone(), inner.clone());
            Closure::<dyn FnMut(Event)>::new(move |_| {
                let mut st = inner.borrow_mut();
                if !matches!(st.state, Some(LinkState::Connecting)) {
                    return;
                }
                st.state = Some(LinkState::Open);
                for t in std::mem::take(&mut st.backlog) {
                    let _ = ws.send_with_str(&t);
                }
            })
        };
        let on_message = {
            let inner = inner.clone();
            Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                if let Some(m) = e.data().as_string().as_deref().and_then(decode) {
                    inner.borrow_mut().inbound.push_back(m);
                }
            })
        };
        let on_close = {
            let inner = inner.clone();
            Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
                let mut st = inner.borrow_mut();
                if matches!(st.state, Some(LinkState::Closed { .. })) {
                    return;
                }
                let reason = if e.reason().is_empty() {
                    format!("signaling connection closed ({})", e.code())
                } else {
                    e.reason()
                };
                st.state = Some(LinkState::Closed {
                    reason,
                    fatal: false,
                });
            })
        };
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        Self {
            ws: Some(ws),
            inner,
            _on_open: Some(on_open),
            _on_message: Some(on_message),
            _on_close: Some(on_close),
        }
    }
}

impl SignalLink for WebSignal {
    fn send(&mut self, message: &SignalClientMessage) {
        let text = encode(message);
        let mut st = self.inner.borrow_mut();
        match (&st.state, &self.ws) {
            (Some(LinkState::Open), Some(ws)) => {
                let _ = ws.send_with_str(&text);
            }
            (Some(LinkState::Connecting), _) => st.backlog.push(text),
            _ => {}
        }
    }

    fn poll(&mut self, out: &mut Vec<SignalServerMessage>) {
        out.extend(self.inner.borrow_mut().inbound.drain(..));
    }

    fn state(&self) -> LinkState {
        self.inner
            .borrow()
            .state
            .clone()
            .unwrap_or(LinkState::Connecting)
    }

    fn close(&mut self) {
        if let Some(ws) = self.ws.take() {
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onclose(None);
            let _ = ws.close();
        }
        let mut st = self.inner.borrow_mut();
        if !matches!(st.state, Some(LinkState::Closed { .. })) {
            st.state = Some(LinkState::Closed {
                reason: "closed".into(),
                fatal: false,
            });
        }
    }
}

impl Drop for WebSignal {
    fn drop(&mut self) {
        self.close();
    }
}
