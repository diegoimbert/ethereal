//! WebCLAP messages over the rings.
//!
//! Worker → Worklet: [`WebclapMsg`], JSON inside `EngineMsg::Webclap` (rare: creation,
//! requests). Worklet → Worker: [`REPORT_PLUGIN`] frames, binary, decoded by
//! [`PluginReport::decode`]: notifications from the instances and the replies to requests
//! (states travel in chunks of at most [`STATE_CHUNK`] bytes, so a big state never needs a
//! frame bigger than the report ring).

use ether_core::NodeKey;
use ether_core::protocol::model::{Base64Bytes, DeviceId};
use serde::{Deserialize, Serialize};

use super::node::NodeSpec;

/// Report tag of every frame here.
pub const REPORT_PLUGIN: u8 = b'P';
/// Largest state piece per frame.
pub const STATE_CHUNK: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WebclapRequest {
    /// Reply [`PluginReport::State`] (chunks).
    State,
    /// Reply [`PluginReport::Values`].
    Values,
}

/// What the Worker asks of the Worklet's instances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WebclapMsg {
    /// Instantiate a node for virtual key `key`.
    Create {
        key: NodeKey,
        device: DeviceId,
        module: String,
        plugin_id: String,
        name: String,
        layout: super::catalog::Layout,
        category: ether_core::protocol::devices::DeviceCategory,
        note_expressions: bool,
        params: Vec<(u32, f64)>,
        always_process: bool,
        state: Option<Base64Bytes>,
    },
    /// Answered after the next block (events sent before it reached the plugin).
    Request {
        key: NodeKey,
        seq: u32,
        request: WebclapRequest,
    },
    AlwaysProcess {
        key: NodeKey,
        on: bool,
    },
}

impl WebclapMsg {
    /// The node spec of a `Create`.
    pub fn spec(&self) -> Option<NodeSpec> {
        match self {
            WebclapMsg::Create {
                device,
                module,
                plugin_id,
                name,
                layout,
                category,
                note_expressions,
                params,
                always_process,
                ..
            } => Some(NodeSpec {
                module: module.clone(),
                plugin_id: plugin_id.clone(),
                name: name.clone(),
                device: device.to_string(),
                layout: *layout,
                category: *category,
                note_expressions: *note_expressions,
                params: params.clone(),
                always_process: *always_process,
            }),
            _ => None,
        }
    }
}

/// One [`REPORT_PLUGIN`] frame.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginReport {
    ParamEdited {
        key: NodeKey,
        id: u32,
        value: f64,
    },
    GestureBegin {
        key: NodeKey,
        id: u32,
    },
    GestureEnd {
        key: NodeKey,
        id: u32,
    },
    StateDirty {
        key: NodeKey,
    },
    ParamsChanged {
        key: NodeKey,
    },
    Crashed {
        key: NodeKey,
        message: String,
    },
    /// `bytes` at `offset` of a state `total` long (`total == 0` with no state:
    /// `has_state` false).
    State {
        key: NodeKey,
        seq: u32,
        has_state: bool,
        total: u32,
        offset: u32,
        bytes: Vec<u8>,
    },
    Values {
        key: NodeKey,
        seq: u32,
        values: Vec<(u32, f64)>,
    },
    /// A request that could not be answered (not instantiated yet, the plugin failed).
    Failed {
        key: NodeKey,
        seq: u32,
        message: String,
    },
}

const K_PARAM: u8 = 1;
const K_BEGIN: u8 = 2;
const K_END: u8 = 3;
const K_DIRTY: u8 = 4;
const K_PARAMS: u8 = 5;
const K_CRASHED: u8 = 6;
const K_STATE: u8 = 10;
const K_VALUES: u8 = 11;
const K_FAILED: u8 = 12;

fn head(kind: u8, key: NodeKey, extra: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(10 + extra);
    v.push(REPORT_PLUGIN);
    v.push(kind);
    v.extend_from_slice(&key.index.to_le_bytes());
    v.extend_from_slice(&key.generation.to_le_bytes());
    v
}

impl PluginReport {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            PluginReport::ParamEdited { key, id, value } => {
                let mut v = head(K_PARAM, *key, 12);
                v.extend_from_slice(&id.to_le_bytes());
                v.extend_from_slice(&value.to_le_bytes());
                v
            }
            PluginReport::GestureBegin { key, id } | PluginReport::GestureEnd { key, id } => {
                let k = if matches!(self, PluginReport::GestureBegin { .. }) {
                    K_BEGIN
                } else {
                    K_END
                };
                let mut v = head(k, *key, 4);
                v.extend_from_slice(&id.to_le_bytes());
                v
            }
            PluginReport::StateDirty { key } => head(K_DIRTY, *key, 0),
            PluginReport::ParamsChanged { key } => head(K_PARAMS, *key, 0),
            PluginReport::Crashed { key, message } => {
                let mut v = head(K_CRASHED, *key, message.len());
                v.extend_from_slice(message.as_bytes());
                v
            }
            PluginReport::State {
                key,
                seq,
                has_state,
                total,
                offset,
                bytes,
            } => {
                let mut v = head(K_STATE, *key, 13 + bytes.len());
                v.extend_from_slice(&seq.to_le_bytes());
                v.push(u8::from(*has_state));
                v.extend_from_slice(&total.to_le_bytes());
                v.extend_from_slice(&offset.to_le_bytes());
                v.extend_from_slice(bytes);
                v
            }
            PluginReport::Values { key, seq, values } => {
                let mut v = head(K_VALUES, *key, 4 + 12 * values.len());
                v.extend_from_slice(&seq.to_le_bytes());
                for (id, value) in values {
                    v.extend_from_slice(&id.to_le_bytes());
                    v.extend_from_slice(&value.to_le_bytes());
                }
                v
            }
            PluginReport::Failed { key, seq, message } => {
                let mut v = head(K_FAILED, *key, 4 + message.len());
                v.extend_from_slice(&seq.to_le_bytes());
                v.extend_from_slice(message.as_bytes());
                v
            }
        }
    }

    /// The frames of a whole state reply.
    pub fn state_frames(key: NodeKey, seq: u32, state: Option<&[u8]>) -> Vec<Vec<u8>> {
        let Some(bytes) = state else {
            return vec![
                PluginReport::State {
                    key,
                    seq,
                    has_state: false,
                    total: 0,
                    offset: 0,
                    bytes: Vec::new(),
                }
                .encode(),
            ];
        };
        let total = bytes.len() as u32;
        let mut out: Vec<Vec<u8>> = bytes
            .chunks(STATE_CHUNK)
            .enumerate()
            .map(|(i, c)| {
                PluginReport::State {
                    key,
                    seq,
                    has_state: true,
                    total,
                    offset: (i * STATE_CHUNK) as u32,
                    bytes: c.to_vec(),
                }
                .encode()
            })
            .collect();
        if out.is_empty() {
            out.push(
                PluginReport::State {
                    key,
                    seq,
                    has_state: true,
                    total: 0,
                    offset: 0,
                    bytes: Vec::new(),
                }
                .encode(),
            );
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&tag, rest) = bytes.split_first()?;
        if tag != REPORT_PLUGIN || rest.len() < 9 {
            return None;
        }
        let kind = rest[0];
        let u32_at = |b: &[u8], i: usize| -> Option<u32> {
            Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
        };
        let f64_at = |b: &[u8], i: usize| -> Option<f64> {
            Some(f64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
        };
        let key = NodeKey {
            index: u32_at(rest, 1)?,
            generation: u32_at(rest, 5)?,
        };
        let body = &rest[9..];
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        Some(match kind {
            K_PARAM => PluginReport::ParamEdited {
                key,
                id: u32_at(body, 0)?,
                value: f64_at(body, 4)?,
            },
            K_BEGIN => PluginReport::GestureBegin {
                key,
                id: u32_at(body, 0)?,
            },
            K_END => PluginReport::GestureEnd {
                key,
                id: u32_at(body, 0)?,
            },
            K_DIRTY => PluginReport::StateDirty { key },
            K_PARAMS => PluginReport::ParamsChanged { key },
            K_CRASHED => PluginReport::Crashed {
                key,
                message: text(body),
            },
            K_STATE => PluginReport::State {
                key,
                seq: u32_at(body, 0)?,
                has_state: *body.get(4)? != 0,
                total: u32_at(body, 5)?,
                offset: u32_at(body, 9)?,
                bytes: body.get(13..)?.to_vec(),
            },
            K_VALUES => {
                let seq = u32_at(body, 0)?;
                let values = body
                    .get(4..)?
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .map(|c| (u32_at(c, 0).unwrap_or(0), f64_at(c, 4).unwrap_or(0.0)))
                    .collect();
                PluginReport::Values { key, seq, values }
            }
            K_FAILED => PluginReport::Failed {
                key,
                seq: u32_at(body, 0)?,
                message: text(body.get(4..)?),
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_round_trip() {
        let key = NodeKey {
            index: 3,
            generation: 9,
        };
        for r in [
            PluginReport::ParamEdited {
                key,
                id: 7,
                value: 0.25,
            },
            PluginReport::GestureBegin { key, id: 1 },
            PluginReport::GestureEnd { key, id: 1 },
            PluginReport::StateDirty { key },
            PluginReport::ParamsChanged { key },
            PluginReport::Crashed {
                key,
                message: "trap: unreachable".into(),
            },
            PluginReport::Values {
                key,
                seq: 4,
                values: vec![(1, 0.5), (2, -3.0)],
            },
            PluginReport::Failed {
                key,
                seq: 5,
                message: "not ready".into(),
            },
        ] {
            assert_eq!(PluginReport::decode(&r.encode()), Some(r));
        }
        assert_eq!(PluginReport::decode(&[REPORT_PLUGIN, 1]), None);
    }

    #[test]
    fn states_travel_in_chunks() {
        let key = NodeKey {
            index: 1,
            generation: 1,
        };
        let state: Vec<u8> = (0..STATE_CHUNK * 2 + 10).map(|i| i as u8).collect();
        let frames = PluginReport::state_frames(key, 2, Some(&state));
        assert_eq!(frames.len(), 3);
        let mut back = vec![0u8; state.len()];
        for f in &frames {
            let Some(PluginReport::State {
                total,
                offset,
                bytes,
                has_state: true,
                ..
            }) = PluginReport::decode(f)
            else {
                panic!("not a state chunk");
            };
            assert_eq!(total as usize, state.len());
            back[offset as usize..offset as usize + bytes.len()].copy_from_slice(&bytes);
        }
        assert_eq!(back, state);
        let none = PluginReport::state_frames(key, 3, None);
        assert!(matches!(
            PluginReport::decode(&none[0]),
            Some(PluginReport::State {
                has_state: false,
                ..
            })
        ));
    }
}
