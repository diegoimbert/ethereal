//! Control channel between the host and the helper: length-prefixed JSON frames over the
//! helper's stdin (requests) and a private duplicate of its stdout (responses).
//!
//! Strictly request/response: the host sends one [`Request`] and waits (with a timeout) for
//! exactly one [`Response`]. The helper's first frame is unsolicited: [`Response::Ready`]
//! (or [`Response::Err`] if the plugin failed to load).

use std::io::{self, Read, Write};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ether_core::plugin::{PluginError, PluginNotification};
use ether_core::protocol::devices::{DeviceDescriptor, ParamInfo};
use ether_core::protocol::model::ParamId;
use serde::{Deserialize, Serialize};

/// Refuse frames larger than this (corrupt stream guard).
const MAX_FRAME: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Request {
    Params,
    ParamValue(u32),
    /// Set a parameter while inactive (CLAP `params.flush`).
    SetParam {
        id: u32,
        value: f64,
    },
    /// Activate the plugin (not yet processing: see [`Request::Attach`]).
    Activate {
        sample_rate: f32,
        max_block_size: u32,
        max_events_per_block: u32,
    },
    /// Map the host-created shared memory + semaphore and start the audio thread.
    Attach {
        shm: String,
        sem: String,
    },
    Deactivate,
    SaveState,
    /// Base64 state blob.
    LoadState(String),
    OpenEditor,
    CloseEditor,
    Poll,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Response {
    Ready {
        descriptor: DeviceDescriptor,
        has_editor: bool,
    },
    Params(Vec<ParamInfo>),
    ParamValue(Option<f64>),
    Activated {
        descriptor: DeviceDescriptor,
        latency: u32,
        inputs: u16,
        outputs: u16,
    },
    /// Base64 state blob.
    State(String),
    Notifications(Vec<Notification>),
    Ok,
    Err(WireError),
}

pub(crate) fn encode_state(state: &[u8]) -> String {
    B64.encode(state)
}

pub(crate) fn decode_state(state: &str) -> Result<Vec<u8>, PluginError> {
    B64.decode(state)
        .map_err(|e| PluginError::Ipc(format!("bad state encoding: {e}")))
}

/// Serializable mirror of [`PluginError`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum WireError {
    NotFound(String),
    Load(String),
    Activation(String),
    State(String),
    NoEditor,
    Crashed(String),
    Ipc(String),
}

impl From<PluginError> for WireError {
    fn from(e: PluginError) -> Self {
        match e {
            PluginError::NotFound(s) => Self::NotFound(s),
            PluginError::Load(s) => Self::Load(s),
            PluginError::Activation(s) => Self::Activation(s),
            PluginError::State(s) => Self::State(s),
            PluginError::NoEditor => Self::NoEditor,
            PluginError::Crashed(s) => Self::Crashed(s),
            PluginError::Ipc(s) => Self::Ipc(s),
        }
    }
}

impl From<WireError> for PluginError {
    fn from(e: WireError) -> Self {
        match e {
            WireError::NotFound(s) => Self::NotFound(s),
            WireError::Load(s) => Self::Load(s),
            WireError::Activation(s) => Self::Activation(s),
            WireError::State(s) => Self::State(s),
            WireError::NoEditor => Self::NoEditor,
            WireError::Crashed(s) => Self::Crashed(s),
            WireError::Ipc(s) => Self::Ipc(s),
        }
    }
}

/// Serializable mirror of [`PluginNotification`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Notification {
    ParamEdited { param: u32, value: f64 },
    GestureBegin { param: u32 },
    GestureEnd { param: u32 },
    LatencyChanged { samples: u32 },
    RestartRequested,
    ParamsChanged,
    StateDirty,
    EditorClosed,
    Crashed { message: String },
}

impl From<PluginNotification> for Notification {
    fn from(n: PluginNotification) -> Self {
        match n {
            PluginNotification::ParamEdited { param, value } => Self::ParamEdited {
                param: param.0,
                value,
            },
            PluginNotification::GestureBegin { param } => Self::GestureBegin { param: param.0 },
            PluginNotification::GestureEnd { param } => Self::GestureEnd { param: param.0 },
            PluginNotification::LatencyChanged { samples } => Self::LatencyChanged { samples },
            PluginNotification::RestartRequested => Self::RestartRequested,
            PluginNotification::ParamsChanged => Self::ParamsChanged,
            PluginNotification::StateDirty => Self::StateDirty,
            PluginNotification::EditorClosed => Self::EditorClosed,
            PluginNotification::Crashed { message } => Self::Crashed { message },
        }
    }
}

impl From<Notification> for PluginNotification {
    fn from(n: Notification) -> Self {
        match n {
            Notification::ParamEdited { param, value } => Self::ParamEdited {
                param: ParamId(param),
                value,
            },
            Notification::GestureBegin { param } => Self::GestureBegin {
                param: ParamId(param),
            },
            Notification::GestureEnd { param } => Self::GestureEnd {
                param: ParamId(param),
            },
            Notification::LatencyChanged { samples } => Self::LatencyChanged { samples },
            Notification::RestartRequested => Self::RestartRequested,
            Notification::ParamsChanged => Self::ParamsChanged,
            Notification::StateDirty => Self::StateDirty,
            Notification::EditorClosed => Self::EditorClosed,
            Notification::Crashed { message } => Self::Crashed { message },
        }
    }
}

/// Write one frame: `u32` little-endian length, then the JSON body.
pub(crate) fn write_frame<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let len = u32::try_from(body.len()).map_err(io::Error::other)?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Read one frame. `Ok(None)` on a clean EOF at a frame boundary.
pub(crate) fn read_frame<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len}"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_roundtrip() {
        let mut buf = Vec::new();
        let reqs = [
            Request::Poll,
            Request::LoadState(encode_state(&[1, 2, 255])),
            Request::Activate {
                sample_rate: 44_100.0,
                max_block_size: 512,
                max_events_per_block: 64,
            },
        ];
        for r in &reqs {
            write_frame(&mut buf, r).unwrap();
        }
        let mut rd = &buf[..];
        for r in &reqs {
            assert_eq!(read_frame::<Request>(&mut rd).unwrap().as_ref(), Some(r));
        }
        assert_eq!(read_frame::<Request>(&mut rd).unwrap(), None);
        assert_eq!(decode_state(&encode_state(&[1, 2, 255])).unwrap(), vec![1, 2, 255]);
    }

    #[test]
    fn notifications_and_errors_mirror() {
        let n = PluginNotification::ParamEdited {
            param: ParamId(3),
            value: 0.5,
        };
        assert_eq!(PluginNotification::from(Notification::from(n.clone())), n);
        let e = PluginError::State("x".into());
        assert_eq!(PluginError::from(WireError::from(e.clone())), e);
    }
}
