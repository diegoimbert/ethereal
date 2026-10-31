//! The Worklet's side of WebCLAP ([`super::proto::WebclapMsg`] handling, used by
//! `crate::worklet::EngineHost`): live nodes by virtual key, requests answered after the
//! block that follows them, and the reports queued for the Worker.
//!
//! Reports are queued here and written to the (lossy, try-once) report ring as space
//! allows, so a notification or a state reply is never dropped, only delayed.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ether_core::{Node, NodeKey, PluginNode};

use super::guest::{GuestNote, with_guest};
use super::node::WebclapNode;
use super::proto::{PluginReport, WebclapMsg, WebclapRequest};
use crate::ring::{RingMemory, RingWriter};

/// Report bytes written per block at most (the rest waits).
const FLUSH_BUDGET: usize = 64 * 1024;
/// A parameter values reply bigger than this (12 bytes a parameter: about 10 900) is
/// answered `Failed`: it could not share the report ring (256 KiB) with anything else.
const MAX_VALUES_BYTES: usize = 128 * 1024;

struct Live {
    handle: u32,
    always: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct WorkletWebclap {
    nodes: BTreeMap<NodeKey, Live>,
    requests: Vec<(NodeKey, u32, WebclapRequest)>,
    outbox: VecDeque<Vec<u8>>,
    notes: Vec<(u32, GuestNote)>,
}

impl WorkletWebclap {
    /// Build the node of a `Create` (the caller adds it to the engine under `key`).
    pub fn create(
        &mut self,
        msg: &WebclapMsg,
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<(NodeKey, Box<dyn Node>), String> {
        let WebclapMsg::Create { key, state, .. } = msg else {
            return Err("not a create".into());
        };
        if !super::guest::has_guest() {
            return Err("webclap plugins can't run here: no host in this audio worklet".into());
        }
        let spec = msg.spec().expect("a create has a spec");
        let node = WebclapNode::new(
            spec,
            state.as_ref().map(|s| &s.0[..]),
            sample_rate,
            max_frames,
        );
        let always = node
            .always_process()
            .expect("webclap nodes have the switch");
        self.nodes.insert(
            *key,
            Live {
                handle: node.handle(),
                always,
            },
        );
        Ok((*key, Box::new(node)))
    }

    /// `Request` and `AlwaysProcess`.
    pub fn apply(&mut self, msg: WebclapMsg) -> Result<(), String> {
        match msg {
            WebclapMsg::Request { key, seq, request } => {
                self.requests.push((key, seq, request));
                Ok(())
            }
            WebclapMsg::AlwaysProcess { key, on } => {
                let live = self
                    .nodes
                    .get(&key)
                    .ok_or_else(|| format!("unknown webclap node {key:?}"))?;
                live.always.store(on, Ordering::Relaxed);
                Ok(())
            }
            WebclapMsg::Create { .. } => Err("create goes through WorkletWebclap::create".into()),
        }
    }

    /// The node `key` was destroyed (or replaced).
    pub fn forget(&mut self, key: NodeKey) {
        self.nodes.remove(&key);
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.outbox.is_empty()
    }

    /// After a block: answer the requests that came before it, queue the instances'
    /// notifications, and write what fits to the report ring.
    pub fn after_render<M: RingMemory>(&mut self, reports: &mut RingWriter<M>) {
        if self.nodes.is_empty() && self.outbox.is_empty() && self.requests.is_empty() {
            return;
        }
        for (key, seq, request) in std::mem::take(&mut self.requests) {
            let Some(live) = self.nodes.get(&key) else {
                self.outbox.push_back(
                    PluginReport::Failed {
                        key,
                        seq,
                        message: "no such webclap instance".into(),
                    }
                    .encode(),
                );
                continue;
            };
            let handle = live.handle;
            match request {
                WebclapRequest::State => {
                    match with_guest(|g| g.save_state(handle)).unwrap_or(Err("no host".into())) {
                        Ok(state) => self.outbox.extend(PluginReport::state_frames(
                            key,
                            seq,
                            state.as_deref(),
                        )),
                        Err(message) => self
                            .outbox
                            .push_back(PluginReport::Failed { key, seq, message }.encode()),
                    }
                }
                WebclapRequest::Values => {
                    let report = match with_guest(|g| g.param_values(handle)).flatten() {
                        Some(values) if 12 * values.len() <= MAX_VALUES_BYTES => {
                            PluginReport::Values { key, seq, values }
                        }
                        Some(values) => PluginReport::Failed {
                            key,
                            seq,
                            message: format!("too many parameters ({})", values.len()),
                        },
                        None => PluginReport::Failed {
                            key,
                            seq,
                            message: "not instantiated yet".into(),
                        },
                    };
                    self.outbox.push_back(report.encode());
                }
            }
        }
        self.notes.clear();
        with_guest(|g| g.take_notifications(&mut self.notes));
        for (handle, note) in self.notes.drain(..) {
            let Some((&key, _)) = self.nodes.iter().find(|(_, l)| l.handle == handle) else {
                continue;
            };
            let report = match note {
                GuestNote::ParamEdited { id, value } => {
                    PluginReport::ParamEdited { key, id, value }
                }
                GuestNote::GestureBegin { id } => PluginReport::GestureBegin { key, id },
                GuestNote::GestureEnd { id } => PluginReport::GestureEnd { key, id },
                GuestNote::StateDirty => PluginReport::StateDirty { key },
                GuestNote::ParamsChanged => PluginReport::ParamsChanged { key },
                GuestNote::Crashed { message } => PluginReport::Crashed { key, message },
            };
            self.outbox.push_back(report.encode());
        }
        flush(&mut self.outbox, reports);
    }
}

/// Write queued reports to the ring while they fit, at most [`FLUSH_BUDGET`] bytes. A report
/// the ring can never hold (a plugin's huge crash message, say) is dropped: waiting at the
/// front, it would hold back every later report for good.
fn flush<M: RingMemory>(outbox: &mut VecDeque<Vec<u8>>, reports: &mut RingWriter<M>) {
    let mut written = 0;
    while let Some(frame) = outbox.front() {
        if crate::ring::FRAME_HEADER + frame.len() > reports.capacity() {
            outbox.pop_front();
            continue;
        }
        if written + frame.len() > FLUSH_BUDGET && written > 0 {
            break;
        }
        if !reports.try_send_now(frame) {
            break;
        }
        written += frame.len();
        outbox.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::{HeapMemory, RingReader};

    #[test]
    fn a_report_bigger_than_the_ring_is_dropped_not_waited_for() {
        let mem = HeapMemory::new(64);
        let (mut w, mut r) = (RingWriter::new(mem.clone()), RingReader::new(mem));
        let mut outbox: VecDeque<Vec<u8>> = VecDeque::from([vec![1; 100], vec![2; 8]]);
        flush(&mut outbox, &mut w);
        assert!(outbox.is_empty());
        let mut got = Vec::new();
        r.drain(usize::MAX, |m| {
            got.push(m.to_vec());
            true
        });
        assert_eq!(got, [vec![2; 8]]);
    }
}
