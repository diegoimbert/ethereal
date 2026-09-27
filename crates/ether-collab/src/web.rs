//! Web client: the browser's `WebSocket`, from the controller's Worker (wasm32).
//!
//! Callbacks run on the Worker's event loop between controller calls; they only push into a
//! shared queue that `poll` drains.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ether_protocol::collab::CollabMessage;
use ether_protocol::remote::{ClientHello, HelloRejection, PROTOCOL_VERSION, ServerHello};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

use crate::wire::{WireFrame, decode_binary, decode_text, encode_frame, session_url};
use crate::{CollabTransport, ConnectRequest, LinkState};

#[derive(Default)]
struct Inner {
    state: Option<LinkState>,
    /// Handshake done (`Welcome` received).
    welcomed: bool,
    inbound: VecDeque<CollabMessage>,
    /// Sent before the link was open.
    backlog: Vec<WireFrame>,
}

pub struct WebClient {
    ws: Option<WebSocket>,
    inner: Rc<RefCell<Inner>>,
    _on_open: Option<Closure<dyn FnMut(Event)>>,
    _on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
    _on_close: Option<Closure<dyn FnMut(CloseEvent)>>,
}

fn send_frame(ws: &WebSocket, frame: &WireFrame) {
    let _ = match frame {
        WireFrame::Text(t) => ws.send_with_str(t),
        WireFrame::Binary(b) => ws.send_with_u8_array(b),
    };
}

impl WebClient {
    pub fn connect(request: &ConnectRequest) -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            state: Some(LinkState::Connecting),
            ..Inner::default()
        }));
        let url = session_url(&request.server, &request.session);
        let ws = match WebSocket::new(&url) {
            Ok(ws) => ws,
            Err(_) => {
                inner.borrow_mut().state = Some(LinkState::Closed {
                    reason: format!("cannot open {url}"),
                    fatal: true,
                });
                return Self {
                    ws: None,
                    inner,
                    _on_open: None,
                    _on_message: None,
                    _on_close: None,
                };
            }
        };
        ws.set_binary_type(BinaryType::Arraybuffer);
        let hello = serde_json::to_string(&ClientHello {
            protocol_version: PROTOCOL_VERSION,
            token: request.token.clone(),
            client: request.client.clone(),
        })
        .expect("hello serializes");
        let on_open = {
            let ws = ws.clone();
            Closure::<dyn FnMut(Event)>::new(move |_| {
                let _ = ws.send_with_str(&hello);
            })
        };
        let on_message = {
            let (ws, inner) = (ws.clone(), inner.clone());
            Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                let data = e.data();
                let mut st = inner.borrow_mut();
                if !st.welcomed {
                    let Some(text) = data.as_string() else { return };
                    match serde_json::from_str::<ServerHello>(&text) {
                        Ok(ServerHello::Welcome { .. }) => {
                            st.welcomed = true;
                            st.state = Some(LinkState::Open);
                            for f in std::mem::take(&mut st.backlog) {
                                send_frame(&ws, &f);
                            }
                        }
                        Ok(ServerHello::Rejected { reason, message }) => {
                            let fatal = matches!(
                                reason,
                                HelloRejection::BadToken
                                    | HelloRejection::UnsupportedVersion
                                    | HelloRejection::Busy
                            );
                            st.state = Some(LinkState::Closed {
                                reason: message,
                                fatal,
                            });
                        }
                        Err(_) => {}
                    }
                    return;
                }
                let decoded = if let Some(text) = data.as_string() {
                    decode_text(&text)
                } else if let Ok(buf) = data.dyn_into::<js_sys::ArrayBuffer>() {
                    decode_binary(&js_sys::Uint8Array::new(&buf).to_vec())
                } else {
                    Err("unexpected frame".into())
                };
                if let Ok(m) = decoded {
                    st.inbound.push_back(m);
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
                let code = e.code();
                let reason = if e.reason().is_empty() {
                    format!("connection closed ({code})")
                } else {
                    e.reason()
                };
                st.state = Some(LinkState::Closed {
                    reason,
                    fatal: (4001..=4003).contains(&code),
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

impl CollabTransport for WebClient {
    fn send(&mut self, message: &CollabMessage) {
        let frame = encode_frame(message);
        let mut st = self.inner.borrow_mut();
        match (&st.state, &self.ws) {
            (Some(LinkState::Open), Some(ws)) => send_frame(ws, &frame),
            (Some(LinkState::Connecting), _) => st.backlog.push(frame),
            _ => {}
        }
    }

    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
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

impl Drop for WebClient {
    fn drop(&mut self) {
        self.close();
    }
}
