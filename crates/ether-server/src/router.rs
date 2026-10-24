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
//! cancelled when it disconnects (`Media::CancelUpload`), and its analysis watches are
//! released (`Analysis::Unwatch`, v0.2: the controller refcounts watches).
//!
//! Upload ids are client-chosen, so they are namespaced per client (`c<client>-<id>`, see
//! [`upload_prefix`]): a client can't append to, import, or cancel another client's
//! upload, and two clients picking the same id don't collide. Every `upload` field of a
//! command is rewritten; `UploadProgress` goes back only to the uploading client, with its
//! own id.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use ether_protocol::analysis::AnalysisCommand;
use ether_protocol::media::{MediaCommand, MediaEvent, MediaSource};
use ether_protocol::model::{DeviceId, GestureId};
use ether_protocol::project::EditCommand;
use ether_protocol::{
    ClientMessage, Command, CommandError, ErrorCode, Event, Reply, ReplyResult, ServerMessage,
};

use crate::frames::{Frame, encode_server};

/// What the router prepends to the upload ids of `client`.
pub fn upload_prefix(client: ClientId) -> String {
    format!("c{client}-")
}

/// The client a namespaced upload id belongs to, and the client's own id.
fn split_upload(upload: &str) -> Option<(ClientId, &str)> {
    let (client, local) = upload.strip_prefix('c')?.split_once('-')?;
    if client.is_empty() || !client.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((client.parse().ok()?, local))
}

/// Prefix every upload id in `command` (any `upload` string field: `Media::BeginUpload`,
/// `UploadChunk`, `CancelUpload`, `MediaSource::Upload`, `BundleSource::Upload`,
/// `Browser::ImportFile`, ...). The hot path (`UploadChunk`, megabytes of data) is typed;
/// other commands are scanned as JSON so a new command carrying an upload id can't
/// bypass the namespace.
fn namespace_uploads(command: &mut Command, prefix: &str) {
    if let Command::Media(
        MediaCommand::BeginUpload { upload, .. }
        | MediaCommand::UploadChunk { upload, .. }
        | MediaCommand::CancelUpload { upload },
    ) = command
    {
        upload.insert_str(0, prefix);
        return;
    }
    fn walk(v: &mut serde_json::Value, prefix: &str) -> bool {
        match v {
            serde_json::Value::Object(o) => {
                let mut changed = false;
                for (k, v) in o.iter_mut() {
                    if k == "upload"
                        && let serde_json::Value::String(s) = v
                    {
                        s.insert_str(0, prefix);
                        changed = true;
                    } else {
                        changed |= walk(v, prefix);
                    }
                }
                changed
            }
            serde_json::Value::Array(a) => a.iter_mut().fold(false, |c, v| walk(v, prefix) | c),
            _ => false,
        }
    }
    let Ok(mut v) = serde_json::to_value(&*command) else {
        return;
    };
    if walk(&mut v, prefix)
        && let Ok(c) = serde_json::from_value(v)
    {
        *command = c;
    }
}

/// Outgoing queue length per client. When a client falls this far behind, high-rate frames
/// (playhead, meters) are dropped for it; if even a reply or an event doesn't fit, its
/// queue is closed and the connection ends once it has written what is queued (its UI
/// would be out of sync anyway).
pub const CLIENT_QUEUE: usize = 4096;
/// Bytes a client's outgoing queue may hold (frames can be large: export chunks, peaks).
pub const CLIENT_QUEUE_BYTES: usize = 64 << 20;
/// Gestures and analysis watches one client may hold open at once.
pub const MAX_OPEN_PER_CLIENT: usize = 1024;

fn frame_len(f: &Frame) -> usize {
    match f {
        Frame::Text(t) => t.len(),
        Frame::Binary(b) => b.len(),
    }
}

/// The sending side of a client's outgoing queue: bounded in frames and in bytes.
#[derive(Clone)]
pub struct QueueTx {
    tx: Sender<Frame>,
    bytes: Arc<AtomicUsize>,
}

/// The receiving side ([`client_queue`]): the connection's writer.
pub struct QueueRx {
    rx: Receiver<Frame>,
    bytes: Arc<AtomicUsize>,
}

/// A client's outgoing queue: [`CLIENT_QUEUE`] frames, [`CLIENT_QUEUE_BYTES`] bytes.
pub fn client_queue() -> (QueueTx, QueueRx) {
    let (tx, rx) = crossbeam_channel::bounded(CLIENT_QUEUE);
    let bytes = Arc::new(AtomicUsize::new(0));
    (
        QueueTx {
            tx,
            bytes: bytes.clone(),
        },
        QueueRx { rx, bytes },
    )
}

impl From<Sender<Frame>> for QueueTx {
    fn from(tx: Sender<Frame>) -> Self {
        Self {
            tx,
            bytes: Arc::default(),
        }
    }
}

impl QueueTx {
    fn try_send(&self, frame: Frame) -> Result<(), TrySendError<Frame>> {
        let n = frame_len(&frame);
        if self.bytes.load(Ordering::Relaxed).saturating_add(n) > CLIENT_QUEUE_BYTES {
            return Err(TrySendError::Full(frame));
        }
        self.bytes.fetch_add(n, Ordering::Relaxed);
        self.tx.try_send(frame).inspect_err(|_| {
            self.bytes.fetch_sub(n, Ordering::Relaxed);
        })
    }
}

impl QueueRx {
    pub fn try_recv(&self) -> Result<Frame, TryRecvError> {
        let f = self.rx.try_recv()?;
        self.bytes.fetch_sub(frame_len(&f), Ordering::Relaxed);
        Ok(f)
    }
}

pub type ClientId = u64;

/// Owner of the replies nobody waits for (requests of departed clients).
const NOBODY: ClientId = 0;

/// Uploads one client may have in progress at once (the controller also caps the total).
pub const MAX_UPLOADS_PER_CLIENT: usize = 4;

/// Why [`Router::add_if`] refused a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

struct Client {
    /// `None` once the queue overflowed (dropping the sender ends the connection).
    tx: Option<QueueTx>,
    /// Client gesture → global gesture.
    gestures: HashMap<u32, u32>,
    /// Uploads started and not finished (imported or cancelled).
    uploads: HashSet<String>,
    /// Analysis watches held (device → count).
    watches: HashMap<DeviceId, u32>,
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
        // UI gesture ids live in the lower half; the controller's own in the upper.
        Self::with_id_base(1, 1)
    }
}

impl Router {
    /// A router whose global request ids start at `request_base` and gesture ids at
    /// `gesture_base` (kept below `0x8000_0000`, the controller's own gestures). For an
    /// engine that other clients drive directly too (the desktop agent bridge,
    /// the agent API): pick bases their ids never reach.
    pub fn with_id_base(request_base: u32, gesture_base: u32) -> Self {
        Self {
            state: Mutex::default(),
            next_client: AtomicU64::new(1),
            next_request: AtomicU32::new(request_base),
            next_gesture: AtomicU32::new(gesture_base & 0x7fff_ffff),
        }
    }

    /// `true` if `reply_id` answers a request this router forwarded (and is still
    /// waiting for).
    pub fn owns_reply(&self, reply_id: u32) -> bool {
        self.lock().pending.contains_key(&reply_id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn client_count(&self) -> usize {
        self.lock().clients.len()
    }

    /// Register a client (after its hello was accepted).
    pub fn add(&self, tx: impl Into<QueueTx>) -> ClientId {
        self.add_if(tx, |_| true).expect("unconditional")
    }

    /// Register a client if `allow(current client count)`; the check and the insert are
    /// atomic, so two hellos racing for the last slot can't both get in.
    pub fn add_if(
        &self,
        tx: impl Into<QueueTx>,
        allow: impl FnOnce(usize) -> bool,
    ) -> Result<ClientId, Full> {
        let mut s = self.lock();
        if !allow(s.clients.len()) {
            return Err(Full);
        }
        let id = self.next_client.fetch_add(1, Ordering::Relaxed);
        s.clients.insert(
            id,
            Client {
                tx: Some(tx.into()),
                gestures: HashMap::new(),
                uploads: HashSet::new(),
                watches: HashMap::new(),
            },
        );
        Ok(id)
    }

    /// Forget a client. Returns the messages to send to the host on its behalf (end its
    /// open gestures, cancel its unfinished uploads); their replies go nowhere (they, and
    /// the replies to the client's requests still in flight, stay [owned](Self::owns_reply)
    /// until they arrive, so an attached engine's other clients never see them).
    pub fn remove(&self, client: ClientId) -> Vec<ClientMessage> {
        let mut s = self.lock();
        for (c, _) in s.pending.values_mut() {
            if *c == client {
                *c = NOBODY;
            }
        }
        let Some(c) = s.clients.remove(&client) else {
            return Vec::new();
        };
        let mut gestures: Vec<u32> = c.gestures.into_values().collect();
        gestures.sort_unstable();
        let mut uploads: Vec<String> = c.uploads.into_iter().collect();
        uploads.sort();
        let mut watches: Vec<DeviceId> = c
            .watches
            .into_iter()
            .flat_map(|(d, n)| std::iter::repeat_n(d, n as usize))
            .collect();
        watches.sort();
        gestures
            .into_iter()
            .map(|g| {
                Command::Edit(EditCommand::EndGesture {
                    gesture: GestureId(g),
                })
            })
            .chain(
                uploads
                    .into_iter()
                    .map(|upload| Command::Media(MediaCommand::CancelUpload { upload })),
            )
            .chain(
                watches
                    .into_iter()
                    .map(|device| Command::Analysis(AnalysisCommand::Unwatch { device })),
            )
            .map(|command| {
                let id = self.next_request.fetch_add(1, Ordering::Relaxed);
                s.pending.insert(id, (NOBODY, 0));
                ClientMessage {
                    id,
                    gesture: None,
                    command,
                }
            })
            .collect()
    }

    /// Rewrite a client's message into the global id/gesture space (and remember where
    /// its reply goes). `Ok(None)` if the client is gone; `Err` = answer the client with
    /// this reply instead (per-client limits).
    pub fn inbound(
        &self,
        client: ClientId,
        mut m: ClientMessage,
    ) -> Result<Option<ClientMessage>, Box<ServerMessage>> {
        let global = self.next_request.fetch_add(1, Ordering::Relaxed);
        let mut s = self.lock();
        let Some(c) = s.clients.get_mut(&client) else {
            return Ok(None);
        };
        namespace_uploads(&mut m.command, &upload_prefix(client));
        if let Command::Media(MediaCommand::BeginUpload { upload, .. }) = &m.command
            && !c.uploads.contains(upload)
            && c.uploads.len() >= MAX_UPLOADS_PER_CLIENT
        {
            return Err(Box::new(ServerMessage::Reply(Reply {
                id: m.id,
                result: ReplyResult::Err {
                    error: CommandError {
                        code: ErrorCode::InvalidState,
                        message: format!(
                            "too many uploads in progress (max {MAX_UPLOADS_PER_CLIENT} per client)"
                        ),
                        key: None,
                    },
                },
            })));
        }
        // Open gestures and watches are released for the client when it leaves, one command
        // each: bounded per client.
        let new_gesture = m.gesture.is_some_and(|g| !c.gestures.contains_key(&g.0));
        let new_watch = matches!(m.command, Command::Analysis(AnalysisCommand::Watch { .. }));
        let watches: usize = c.watches.values().map(|&n| n as usize).sum();
        if (new_gesture && c.gestures.len() >= MAX_OPEN_PER_CLIENT)
            || (new_watch && watches >= MAX_OPEN_PER_CLIENT)
        {
            return Err(Box::new(ServerMessage::Reply(Reply {
                id: m.id,
                result: ReplyResult::Err {
                    error: CommandError {
                        code: ErrorCode::InvalidState,
                        message: format!(
                            "too many open gestures or watches (max {MAX_OPEN_PER_CLIENT} per client)"
                        ),
                        key: None,
                    },
                },
            })));
        }
        // An `Unwatch` this client doesn't hold is a no-op for it (never release another
        // client's watch).
        if let Command::Analysis(AnalysisCommand::Unwatch { device }) = &m.command
            && !c.watches.contains_key(device)
        {
            return Err(Box::new(ServerMessage::Reply(Reply {
                id: m.id,
                result: ReplyResult::Ok {
                    value: ether_protocol::ReplyValue::Unit,
                },
            })));
        }
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
            Command::Edit(
                EditCommand::EndGesture { gesture } | EditCommand::CancelGesture { gesture },
            ) => {
                let local = gesture.0;
                *gesture = GestureId(map(local));
                c.gestures.remove(&local);
            }
            Command::Media(MediaCommand::BeginUpload { upload, .. }) => {
                c.uploads.insert(upload.clone());
            }
            Command::Analysis(AnalysisCommand::Watch { device }) => {
                *c.watches.entry(*device).or_insert(0) += 1;
            }
            Command::Analysis(AnalysisCommand::Unwatch { device }) => {
                match c.watches.get_mut(device) {
                    Some(n) if *n > 1 => *n -= 1,
                    Some(_) => {
                        c.watches.remove(device);
                    }
                    None => {}
                }
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
        Ok(Some(m))
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
                // Error messages may quote a namespaced upload id: show the client its own.
                if let ReplyResult::Err { error } = &mut r.result {
                    error.message = error.message.replace(&upload_prefix(client), "");
                }
                let frame = encode_server(&ServerMessage::Reply(r));
                if let Some(c) = s.clients.get_mut(&client) {
                    reliable(&mut c.tx, frame);
                }
            }
            ServerMessage::Event(Event::Media {
                event: MediaEvent::UploadProgress { upload, received },
            }) => {
                // Only the uploading client, with its own id.
                let Some((client, local)) = split_upload(&upload) else {
                    return;
                };
                let frame = encode_server(&ServerMessage::Event(Event::Media {
                    event: MediaEvent::UploadProgress {
                        upload: local.to_string(),
                        received,
                    },
                }));
                if let Some(c) = s.clients.get_mut(&client) {
                    reliable(&mut c.tx, frame);
                }
            }
            mut m => {
                if let ServerMessage::Event(Event::Media {
                    event:
                        MediaEvent::PreviewStarted { source } | MediaEvent::PreviewEnded { source, .. },
                }) = &mut m
                    && let MediaSource::Upload { upload } = source
                    && let Some((_, local)) = split_upload(upload)
                {
                    *upload = local.to_string();
                }
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
fn reliable(tx: &mut Option<QueueTx>, frame: Frame) {
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
    use ether_protocol::ReplyValue;
    use ether_protocol::transport::TransportCommand;

    fn msg(id: u32, gesture: Option<u32>, command: Command) -> ClientMessage {
        ClientMessage {
            id,
            gesture: gesture.map(GestureId),
            command,
        }
    }

    impl Router {
        fn inbound_ok(&self, c: ClientId, m: ClientMessage) -> Option<ClientMessage> {
            self.inbound(c, m).expect("accepted")
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
        let ga = r.inbound_ok(a, msg(1, Some(5), play.clone())).unwrap();
        let gb = r.inbound_ok(b, msg(1, Some(5), play.clone())).unwrap();
        assert_ne!(ga.id, gb.id, "global request ids");
        assert_ne!(
            ga.gesture, gb.gesture,
            "gestures never merge across clients"
        );
        // Same client gesture maps to the same global gesture until EndGesture.
        let ga2 = r.inbound_ok(a, msg(2, Some(5), play)).unwrap();
        assert_eq!(ga.gesture, ga2.gesture);
        let end = r
            .inbound_ok(
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
            key: None,
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
    fn disconnect_releases_analysis_watches_and_unheld_unwatch_is_a_no_op() {
        let r = Router::default();
        let (t, _rx) = crossbeam_channel::bounded(16);
        let (t2, _rx2) = crossbeam_channel::bounded(16);
        let a = r.add(t);
        let b = r.add(t2);
        let d = DeviceId(ether_protocol::model::Ulid(7));
        let watch = |r: &Router, c, cmd| r.inbound(c, msg(1, None, Command::Analysis(cmd)));
        for _ in 0..2 {
            assert!(watch(&r, a, AnalysisCommand::Watch { device: d }).is_ok());
        }
        // `b` never watched `d`: its Unwatch is answered locally, not forwarded.
        assert!(watch(&r, b, AnalysisCommand::Unwatch { device: d }).is_err());
        assert!(watch(&r, a, AnalysisCommand::Unwatch { device: d }).is_ok());
        let released: Vec<Command> = r.remove(a).into_iter().map(|m| m.command).collect();
        assert_eq!(
            released,
            vec![Command::Analysis(AnalysisCommand::Unwatch { device: d })]
        );
    }

    #[test]
    fn cancel_gesture_maps_like_end_gesture() {
        // CancelGesture names the client's gesture: it reaches the engine as the
        // global one, which is then forgotten (a disconnect doesn't end it again).
        let r = Router::default();
        let (t, _rx) = crossbeam_channel::bounded(16);
        let c = r.add(t);
        let global = r
            .inbound_ok(
                c,
                msg(1, Some(7), Command::Transport(TransportCommand::Stop)),
            )
            .unwrap()
            .gesture
            .unwrap();
        let cancel = r
            .inbound_ok(
                c,
                msg(
                    2,
                    None,
                    Command::Edit(EditCommand::CancelGesture {
                        gesture: GestureId(7),
                    }),
                ),
            )
            .unwrap();
        assert_eq!(
            cancel.command,
            Command::Edit(EditCommand::CancelGesture { gesture: global })
        );
        assert!(r.remove(c).is_empty());
    }

    #[test]
    fn disconnect_ends_gestures_and_cancels_unfinished_uploads() {
        let r = Router::default();
        let (t, _rx) = crossbeam_channel::bounded(16);
        let c = r.add(t);
        let open = r
            .inbound_ok(
                c,
                msg(9, Some(5), Command::Transport(TransportCommand::Stop)),
            )
            .unwrap()
            .gesture
            .unwrap();
        for u in ["u1", "u2", "u3"] {
            r.inbound_ok(
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
        r.inbound_ok(
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
                Command::Edit(EditCommand::EndGesture { gesture: open }),
                Command::Media(MediaCommand::CancelUpload {
                    upload: "c1-u1".into()
                }),
                Command::Media(MediaCommand::CancelUpload {
                    upload: "c1-u3".into()
                }),
            ]
        );
        assert_eq!(r.client_count(), 0);
        assert!(
            r.inbound_ok(c, msg(3, None, Command::Transport(TransportCommand::Stop)))
                .is_none()
        );
    }

    #[test]
    fn upload_ids_are_namespaced_in_every_command_and_progress_goes_home() {
        let r = Router::default();
        let (ta, ra) = crossbeam_channel::bounded(16);
        let (tb, rb) = crossbeam_channel::bounded(16);
        let a = r.add(ta);
        let b = r.add(tb);
        let cmd = |c: ClientId, command: Command| r.inbound_ok(c, msg(1, None, command)).unwrap();
        let begin = cmd(
            a,
            Command::Media(MediaCommand::BeginUpload {
                upload: "x".into(),
                name: "a.wav".into(),
                size: 1.0,
            }),
        );
        assert!(
            matches!(begin.command, Command::Media(MediaCommand::BeginUpload { upload, .. }) if upload == "c1-x")
        );
        // Upload ids nested anywhere in a command (scanned as JSON).
        for json in [
            r#"{"domain":"Media","command":{"type":"Import","id":"01J8ZK3V9Q4N2B7XK6M1T0R5AS","source":{"type":"Upload","upload":"x"}}}"#,
            r#"{"domain":"Browser","command":{"type":"ImportFile","root":"r","path":"a.wav","upload":"x"}}"#,
        ] {
            let command: Command = serde_json::from_str(json).unwrap();
            let out = serde_json::to_string(&cmd(b, command)).unwrap();
            assert!(out.contains(r#""upload":"c2-x""#), "{out}");
        }
        // Progress: only to the uploader, with its own id; others never see it.
        r.outbound(ServerMessage::Event(Event::Media {
            event: MediaEvent::UploadProgress {
                upload: "c1-x".into(),
                received: 1.0,
            },
        }));
        let got = texts(&ra);
        assert_eq!(got.len(), 1);
        assert!(got[0].contains(r#""upload":"x""#), "{}", got[0]);
        assert!(texts(&rb).is_empty());
        // An error reply quoting the id shows the requester its own.
        let m = r
            .inbound_ok(a, msg(7, None, Command::Transport(TransportCommand::Stop)))
            .unwrap();
        r.outbound(ServerMessage::Reply(Reply {
            id: m.id,
            result: ReplyResult::Err {
                error: CommandError {
                    code: ErrorCode::NotFound,
                    message: "upload c1-x".into(),
                    key: None,
                },
            },
        }));
        let got = texts(&ra);
        assert!(got[0].contains(r#""message":"upload x""#), "{}", got[0]);
        assert_eq!(split_upload("c12-a-b"), Some((12, "a-b")));
        assert_eq!(split_upload("cx-a"), None);
        assert_eq!(split_upload("c-a"), None);
    }

    #[test]
    fn per_client_upload_cap_and_atomic_admission() {
        let r = Router::default();
        let (t, _rx) = crossbeam_channel::bounded(16);
        let c = r.add(t);
        let begin = |i: u32| {
            msg(
                i,
                None,
                Command::Media(MediaCommand::BeginUpload {
                    upload: format!("u{i}"),
                    name: "a.wav".into(),
                    size: 1.0,
                }),
            )
        };
        for i in 0..MAX_UPLOADS_PER_CLIENT as u32 {
            r.inbound_ok(c, begin(i));
        }
        let Err(reply) = r.inbound(c, begin(99)) else {
            panic!("over the per-client cap")
        };
        let ServerMessage::Reply(reply) = *reply else {
            panic!("over the per-client cap")
        };
        assert_eq!(reply.id, 99);
        // Restarting an upload it already has is fine; another client has its own budget.
        r.inbound_ok(c, begin(0));
        let (t2, _rx2) = crossbeam_channel::bounded(16);
        let c2 = r.add(t2);
        r.inbound_ok(c2, begin(50));
        // Admission is checked under the same lock as the insert.
        let (t3, _rx3) = crossbeam_channel::bounded(16);
        assert_eq!(r.add_if(t3, |n| n < 2), Err(Full));
        assert_eq!(r.client_count(), 2);
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
            key: None,
        }));
        r.outbound(ServerMessage::Event(ether_protocol::Event::Notification {
            level: ether_protocol::NotificationLevel::Info,
            message: "x".into(),
            key: None,
        }));
        r.outbound(ServerMessage::Event(ether_protocol::Event::Notification {
            level: ether_protocol::NotificationLevel::Info,
            message: "x".into(),
            key: None,
        }));
        assert_eq!(rx.try_iter().count(), 2);
        // The queue overflowed on a reliable frame: the sender is gone, so the connection
        // sees the end of its queue.
        assert!(matches!(
            rx.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn open_gestures_and_watches_are_bounded_per_client() {
        let r = Router::default();
        let (t, _rx) = crossbeam_channel::bounded(16);
        let c = r.add(t);
        let stop = || Command::Transport(TransportCommand::Stop);
        for g in 0..MAX_OPEN_PER_CLIENT as u32 {
            r.inbound_ok(c, msg(g, Some(g), stop()));
        }
        assert!(
            r.inbound(c, msg(1, Some(1), stop())).is_ok(),
            "a known gesture"
        );
        assert!(
            r.inbound(c, msg(2, Some(99_999), stop())).is_err(),
            "one more"
        );
        let d = DeviceId(ether_protocol::model::Ulid(7));
        let watch = |id| {
            msg(
                id,
                None,
                Command::Analysis(AnalysisCommand::Watch { device: d }),
            )
        };
        for i in 0..MAX_OPEN_PER_CLIENT as u32 {
            r.inbound_ok(c, watch(i));
        }
        assert!(r.inbound(c, watch(5)).is_err());
    }

    #[test]
    fn the_outgoing_queue_is_bounded_in_bytes() {
        let (tx, rx) = client_queue();
        let big = || Frame::Binary(vec![0; CLIENT_QUEUE_BYTES / 2 + 1]);
        assert!(tx.try_send(big()).is_ok());
        assert!(matches!(tx.try_send(big()), Err(TrySendError::Full(_))));
        assert!(tx.try_send(Frame::Text("small".into())).is_ok());
        rx.try_recv().unwrap();
        assert!(tx.try_send(big()).is_ok(), "room again once written");
    }
}
