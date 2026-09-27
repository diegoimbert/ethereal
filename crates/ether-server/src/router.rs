//! Fan-out of the host's `ServerMessage`s to the connected clients.
//!
//! One controller serves every client. Each client numbers its requests (and gestures)
//! itself, so the router rewrites them into one global space before they reach the host
//! and maps replies back:
//! - `Reply`: only to the client that sent the request, with its own id;
//! - everything else (patches, events, playhead, meters): to every client.
//!
//! Gestures are remapped per client so two clients' drags never merge into one undo step
//! (`ClientMessage::gesture` and `Edit::EndGesture`). Uploads a client started are
//! cancelled when it disconnects (`Media::CancelUpload`).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crossbeam_channel::{Sender, TrySendError};
use ether_protocol::media::{MediaCommand, MediaSource};
use ether_protocol::model::GestureId;
use ether_protocol::project::EditCommand;
use ether_protocol::{ClientMessage, Command, ServerMessage};

use crate::frames::{Frame, encode_server};

/// Outgoing queue length per client. When a client falls this far behind, high-rate frames
/// (playhead, meters) are dropped for it; if even a reply or an event doesn't fit, its
/// queue is closed and the connection ends once it has written what is queued (its UI
/// would be out of sync anyway).
pub const CLIENT_QUEUE: usize = 4096;

pub type ClientId = u64;

struct Client {
    /// `None` once the queue overflowed (dropping the sender ends the connection).
    tx: Option<Sender<Frame>>,
    /// Client gesture → global gesture.
    gestures: HashMap<u32, u32>,
    /// Uploads started and not finished (imported or cancelled).
    uploads: HashSet<String>,
}

#[derive(Default)]
struct State {
    clients: HashMap<ClientId, Client>,
    /// Global request id → (client, the client's request id).
    pending: HashMap<u32, (ClientId, u32)>,
}

pub struct Router {
    state: Mutex<State>,
    next_client: AtomicU64,
    next_request: AtomicU32,
    next_gesture: AtomicU32,
}

impl Default for Router {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            next_client: AtomicU64::new(1),
            next_request: AtomicU32::new(1),
            // UI gesture ids live in the lower half; the controller's own in the upper.
            next_gesture: AtomicU32::new(1),
        }
    }
}

impl Router {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn client_count(&self) -> usize {
        self.lock().clients.len()
    }

    /// Register a client (after its hello was accepted).
    pub fn add(&self, tx: Sender<Frame>) -> ClientId {
        let id = self.next_client.fetch_add(1, Ordering::Relaxed);
        self.lock().clients.insert(
            id,
            Client {
                tx: Some(tx),
                gestures: HashMap::new(),
                uploads: HashSet::new(),
            },
        );
        id
    }

    /// Forget a client. Returns the messages to send to the host on its behalf (cancel its
    /// unfinished uploads); their replies go nowhere.
    pub fn remove(&self, client: ClientId) -> Vec<ClientMessage> {
        let mut s = self.lock();
        s.pending.retain(|_, (c, _)| *c != client);
        let Some(c) = s.clients.remove(&client) else {
            return Vec::new();
        };
        drop(s);
        let mut uploads: Vec<String> = c.uploads.into_iter().collect();
        uploads.sort();
        uploads
            .into_iter()
            .map(|upload| ClientMessage {
                id: self.next_request.fetch_add(1, Ordering::Relaxed),
                gesture: None,
                command: Command::Media(MediaCommand::CancelUpload { upload }),
            })
            .collect()
    }

    /// Rewrite a client's message into the global id/gesture space (and remember where
    /// its reply goes). `None` if the client is gone.
    pub fn inbound(&self, client: ClientId, mut m: ClientMessage) -> Option<ClientMessage> {
        let global = self.next_request.fetch_add(1, Ordering::Relaxed);
        let mut s = self.lock();
        let c = s.clients.get_mut(&client)?;
        let next_gesture = &self.next_gesture;
        let mut map = |g: u32| {
            *c.gestures
                .entry(g)
                .or_insert_with(|| next_gesture.fetch_add(1, Ordering::Relaxed) & 0x7fff_ffff)
        };
        if let Some(GestureId(g)) = m.gesture {
            m.gesture = Some(GestureId(map(g)));
        }
        match &mut m.command {
            Command::Edit(EditCommand::EndGesture { gesture }) => {
                let local = gesture.0;
                *gesture = GestureId(map(local));
                c.gestures.remove(&local);
            }
            Command::Media(MediaCommand::BeginUpload { upload, .. }) => {
                c.uploads.insert(upload.clone());
            }
            Command::Media(MediaCommand::CancelUpload { upload })
            | Command::Media(MediaCommand::Import {
                source: MediaSource::Upload { upload },
                ..
            }) => {
                c.uploads.remove(upload.as_str());
            }
            _ => {}
        }
        s.pending.insert(global, (client, m.id));
        m.id = global;
        Some(m)
    }

    /// Route one message from the host (called on the controller thread).
    pub fn outbound(&self, m: ServerMessage) {
        let mut s = self.lock();
        match m {
            ServerMessage::Reply(mut r) => {
                let Some((client, id)) = s.pending.remove(&r.id) else {
                    return; // a request of a client that left, or an internal one
                };
                r.id = id;
                let frame = encode_server(&ServerMessage::Reply(r));
                if let Some(c) = s.clients.get_mut(&client) {
                    reliable(&mut c.tx, frame);
                }
            }
            m => {
                let lossy = matches!(m, ServerMessage::Playhead(_) | ServerMessage::Meters(_));
                if s.clients.is_empty() {
                    return;
                }
                let frame = encode_server(&m);
                for c in s.clients.values_mut() {
                    if lossy {
                        if let Some(tx) = &c.tx {
                            let _ = tx.try_send(frame.clone());
                        }
                    } else {
                        reliable(&mut c.tx, frame.clone());
                    }
                }
            }
        }
    }
}

/// Queue a frame that must not be dropped. On overflow the queue is closed (the
/// connection ends after writing what it has).
fn reliable(tx: &mut Option<Sender<Frame>>, frame: Frame) {
    if let Some(t) = tx
        && let Err(TrySendError::Full(_)) = t.try_send(frame)
    {
        tracing::warn!("client too slow: disconnecting it");
        *tx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::transport::TransportCommand;
    use ether_protocol::{Reply, ReplyResult, ReplyValue};

    fn msg(id: u32, gesture: Option<u32>, command: Command) -> ClientMessage {
        ClientMessage {
            id,
            gesture: gesture.map(GestureId),
            command,
        }
    }

    fn texts(rx: &crossbeam_channel::Receiver<Frame>) -> Vec<String> {
        rx.try_iter()
            .map(|o| match o {
                Frame::Text(t) => t,
                o => panic!("{o:?}"),
            })
            .collect()
    }

    #[test]
    fn replies_go_to_the_requester_and_events_to_everyone() {
        let r = Router::default();
        let (ta, ra) = crossbeam_channel::bounded(16);
        let (tb, rb) = crossbeam_channel::bounded(16);
        let a = r.add(ta);
        let b = r.add(tb);
        let play = Command::Transport(TransportCommand::Play);
        let ga = r.inbound(a, msg(1, Some(5), play.clone())).unwrap();
        let gb = r.inbound(b, msg(1, Some(5), play.clone())).unwrap();
        assert_ne!(ga.id, gb.id, "global request ids");
        assert_ne!(
            ga.gesture, gb.gesture,
            "gestures never merge across clients"
        );
        // Same client gesture maps to the same global gesture until EndGesture.
        let ga2 = r.inbound(a, msg(2, Some(5), play)).unwrap();
        assert_eq!(ga.gesture, ga2.gesture);
        let end = r
            .inbound(
                a,
                msg(
                    3,
                    None,
                    Command::Edit(EditCommand::EndGesture {
                        gesture: GestureId(5),
                    }),
                ),
            )
            .unwrap();
        assert_eq!(
            end.command,
            Command::Edit(EditCommand::EndGesture {
                gesture: ga.gesture.unwrap()
            })
        );
        r.outbound(ServerMessage::Event(ether_protocol::Event::Notification {
            level: ether_protocol::NotificationLevel::Info,
            message: "hi".into(),
        }));
        r.outbound(ServerMessage::Reply(Reply {
            id: gb.id,
            result: ReplyResult::Ok {
                value: ReplyValue::Unit,
            },
        }));
        let (a_got, b_got) = (texts(&ra), texts(&rb));
        assert_eq!(a_got.len(), 1);
        assert_eq!(b_got.len(), 2);
        assert!(
            b_got[1].contains(r#""id":1"#),
            "client's own id: {}",
            b_got[1]
        );
    }

    #[test]
    fn disconnect_cancels_unfinished_uploads() {
        let r = Router::default();
        let (t, _rx) = crossbeam_channel::bounded(16);
        let c = r.add(t);
        for u in ["u1", "u2", "u3"] {
            r.inbound(
                c,
                msg(
                    1,
                    None,
                    Command::Media(MediaCommand::BeginUpload {
                        upload: u.into(),
                        name: "a.wav".into(),
                        size: 1.0,
                    }),
                ),
            );
        }
        r.inbound(
            c,
            msg(
                2,
                None,
                Command::Media(MediaCommand::CancelUpload {
                    upload: "u2".into(),
                }),
            ),
        );
        let cancels: Vec<Command> = r.remove(c).into_iter().map(|m| m.command).collect();
        assert_eq!(
            cancels,
            vec![
                Command::Media(MediaCommand::CancelUpload {
                    upload: "u1".into()
                }),
                Command::Media(MediaCommand::CancelUpload {
                    upload: "u3".into()
                }),
            ]
        );
        assert_eq!(r.client_count(), 0);
        assert!(
            r.inbound(c, msg(3, None, Command::Transport(TransportCommand::Stop)))
                .is_none()
        );
    }

    #[test]
    fn slow_clients_lose_frames_not_replies() {
        let r = Router::default();
        let (t, rx) = crossbeam_channel::bounded(2);
        r.add(t);
        for _ in 0..5 {
            r.outbound(ServerMessage::Meters(ether_protocol::meters::MeterFrame {
                tracks: Vec::new(),
                cpu_load: 0.0,
            }));
        }
        assert_eq!(rx.try_iter().count(), 2);
        r.outbound(ServerMessage::Event(ether_protocol::Event::Notification {
            level: ether_protocol::NotificationLevel::Info,
            message: "x".into(),
        }));
        r.outbound(ServerMessage::Event(ether_protocol::Event::Notification {
            level: ether_protocol::NotificationLevel::Info,
            message: "x".into(),
        }));
        r.outbound(ServerMessage::Event(ether_protocol::Event::Notification {
            level: ether_protocol::NotificationLevel::Info,
            message: "x".into(),
        }));
        assert_eq!(rx.try_iter().count(), 2);
        // The queue overflowed on a reliable frame: the sender is gone, so the connection
        // sees the end of its queue.
        assert!(matches!(
            rx.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        ));
    }
}
