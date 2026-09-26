//! Messages between the controller Worker and the AudioWorklet (over [`crate::ring`]).
//!
//! - Worker → Worklet: [`EngineMsg`]. Plain-data engine calls (the `EngineBridge` surface).
//!   Encoded as JSON (tag `b'J'`), except decoded media, which is raw planar `f32`
//!   (tag `b'M'`) so a multi-minute file doesn't go through a text format.
//! - Worklet → Worker: [`EngineReport`] (playhead, max-held meters, diagnostics; compact
//!   binary encoded into a reused buffer so the audio thread doesn't allocate) and
//!   [`REPORT_ERROR`] text messages (compile errors etc.).

use std::sync::Arc;

use ether_core::protocol::meters::TrackMeter;
use ether_core::protocol::model::{BuiltinDevice, MediaId, ParamId, TrackId};
use ether_core::{NodeKey, ParamChange, PlayheadState, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;
use serde::{Deserialize, Serialize};

const TAG_JSON: u8 = b'J';
const TAG_MEDIA: u8 = b'M';

/// One engine call, Worker → Worklet. Node keys are *virtual*: allocated by the Worker
/// (it must answer `create_builtin` synchronously) and mapped to real engine keys by the
/// Worklet, which owns the `EngineHandle`.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineMsg {
    CreateBuiltin {
        key: NodeKey,
        device: BuiltinDevice,
        params: Vec<(ParamId, f64)>,
    },
    DestroyNode {
        key: NodeKey,
    },
    /// Decoded audio at the engine sample rate.
    LoadMedia {
        media: MediaId,
        audio: Arc<DecodedAudio>,
    },
    UnloadMedia {
        media: MediaId,
    },
    Publish {
        graph: Box<RenderGraphDesc>,
    },
    SetParam {
        change: ParamChange,
    },
    Transport {
        control: TransportControl,
    },
}

/// The JSON-encoded subset of [`EngineMsg`].
#[derive(Serialize, Deserialize)]
enum JsonMsg {
    CreateBuiltin {
        key: NodeKey,
        device: BuiltinDevice,
        params: Vec<(ParamId, f64)>,
    },
    DestroyNode {
        key: NodeKey,
    },
    UnloadMedia {
        media: MediaId,
    },
    Publish {
        graph: Box<RenderGraphDesc>,
    },
    SetParam {
        change: ParamChange,
    },
    Transport {
        control: TransportControl,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DecodeError {
    #[error("empty message")]
    Empty,
    #[error("unknown tag {0}")]
    Tag(u8),
    #[error("truncated message")]
    Truncated,
    #[error("json: {0}")]
    Json(String),
}

impl EngineMsg {
    pub fn encode(&self) -> Vec<u8> {
        let json = match self.clone() {
            EngineMsg::LoadMedia { media, audio } => return encode_media(media, &audio),
            EngineMsg::CreateBuiltin {
                key,
                device,
                params,
            } => JsonMsg::CreateBuiltin {
                key,
                device,
                params,
            },
            EngineMsg::DestroyNode { key } => JsonMsg::DestroyNode { key },
            EngineMsg::UnloadMedia { media } => JsonMsg::UnloadMedia { media },
            EngineMsg::Publish { graph } => JsonMsg::Publish { graph },
            EngineMsg::SetParam { change } => JsonMsg::SetParam { change },
            EngineMsg::Transport { control } => JsonMsg::Transport { control },
        };
        let mut out = vec![TAG_JSON];
        serde_json::to_writer(&mut out, &json).expect("engine messages serialize");
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (&tag, rest) = bytes.split_first().ok_or(DecodeError::Empty)?;
        match tag {
            TAG_MEDIA => decode_media(rest),
            TAG_JSON => {
                let msg: JsonMsg =
                    serde_json::from_slice(rest).map_err(|e| DecodeError::Json(e.to_string()))?;
                Ok(match msg {
                    JsonMsg::CreateBuiltin {
                        key,
                        device,
                        params,
                    } => EngineMsg::CreateBuiltin {
                        key,
                        device,
                        params,
                    },
                    JsonMsg::DestroyNode { key } => EngineMsg::DestroyNode { key },
                    JsonMsg::UnloadMedia { media } => EngineMsg::UnloadMedia { media },
                    JsonMsg::Publish { graph } => EngineMsg::Publish { graph },
                    JsonMsg::SetParam { change } => EngineMsg::SetParam { change },
                    JsonMsg::Transport { control } => EngineMsg::Transport { control },
                })
            }
            t => Err(DecodeError::Tag(t)),
        }
    }
}

// Media: [M][id_len u8][id utf8][sample_rate u32][channels u16][frames u32][f32 LE planar].
fn encode_media(media: MediaId, audio: &DecodedAudio) -> Vec<u8> {
    let id = media.to_string();
    let frames = audio.frames();
    let mut out = Vec::with_capacity(12 + id.len() + audio.channels.len() * frames * 4);
    out.push(TAG_MEDIA);
    out.push(id.len() as u8);
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(&audio.sample_rate.to_le_bytes());
    out.extend_from_slice(&(audio.channels.len() as u16).to_le_bytes());
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    for ch in &audio.channels {
        for s in &ch[..frames] {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_bits(self.u64()?))
    }
}

fn decode_media(bytes: &[u8]) -> Result<EngineMsg, DecodeError> {
    let mut c = Cursor(bytes);
    let id_len = c.u8()? as usize;
    let id = std::str::from_utf8(c.take(id_len)?).map_err(|e| DecodeError::Json(e.to_string()))?;
    let media: MediaId = id
        .parse()
        .map_err(|_| DecodeError::Json(format!("media id {id}")))?;
    let sample_rate = c.u32()?;
    let channels = c.u16()? as usize;
    let frames = c.u32()? as usize;
    let data = c.take(channels * frames * 4)?;
    let channels = (0..channels)
        .map(|ch| {
            data[ch * frames * 4..(ch + 1) * frames * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect()
        })
        .collect::<Vec<Vec<f32>>>();
    Ok(EngineMsg::LoadMedia {
        media,
        audio: Arc::new(DecodedAudio {
            sample_rate,
            channels,
        }),
    })
}

/// Tag of a report message (Worklet → Worker).
pub const REPORT_STATE: u8 = b'R';
/// Tag of an error text message (Worklet → Worker).
pub const REPORT_ERROR: u8 = b'E';

/// Engine outputs since the previous report, Worklet → Worker.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineReport {
    pub playhead: PlayheadState,
    pub meters: Vec<TrackMeter>,
    pub event_overflow: bool,
    pub underruns: u32,
    /// Blocks rendered since the Worklet started (diagnostics / liveness).
    pub blocks: u64,
}

// [R][flags u8][position f64][seconds f64][bpm f64][sample_time u64][blocks u64]
// [underruns u32][count u16] then per meter: [track u128 LE][peak f32 x2][rms f32 x2][clipped u8]
const METER_BYTES: usize = 16 + 16 + 1;
const REPORT_FIXED: usize = 1 + 1 + 8 * 5 + 4 + 2;

impl EngineReport {
    /// Encoded size (to pre-size buffers).
    pub fn encoded_len(meters: usize) -> usize {
        REPORT_FIXED + meters * METER_BYTES
    }

    /// Encode into `out` (cleared first). Doesn't allocate if `out` has enough capacity.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.push(REPORT_STATE);
        let p = &self.playhead;
        out.push(p.playing as u8 | (p.recording as u8) << 1 | (self.event_overflow as u8) << 2);
        out.extend_from_slice(&p.position.0.to_le_bytes());
        out.extend_from_slice(&p.seconds.to_le_bytes());
        out.extend_from_slice(&p.bpm.to_le_bytes());
        out.extend_from_slice(&p.sample_time.to_le_bytes());
        out.extend_from_slice(&self.blocks.to_le_bytes());
        out.extend_from_slice(&self.underruns.to_le_bytes());
        let n = self.meters.len().min(u16::MAX as usize);
        out.extend_from_slice(&(n as u16).to_le_bytes());
        for m in &self.meters[..n] {
            out.extend_from_slice(&m.track.0.0.to_le_bytes());
            for v in m.peak.iter().chain(&m.rms) {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.push(m.clipped as u8);
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut c = Cursor(bytes);
        if c.u8()? != REPORT_STATE {
            return Err(DecodeError::Tag(bytes[0]));
        }
        let flags = c.u8()?;
        let playhead = PlayheadState {
            playing: flags & 1 != 0,
            recording: flags & 2 != 0,
            position: ether_core::protocol::model::Beats(c.f64()?),
            seconds: c.f64()?,
            bpm: c.f64()?,
            sample_time: c.u64()?,
        };
        let blocks = c.u64()?;
        let underruns = c.u32()?;
        let n = c.u16()? as usize;
        let mut meters = Vec::with_capacity(n);
        for _ in 0..n {
            let id = u128::from_le_bytes(c.take(16)?.try_into().unwrap());
            let peak = [c.f32()?, c.f32()?];
            let rms = [c.f32()?, c.f32()?];
            let clipped = c.u8()? != 0;
            meters.push(TrackMeter {
                track: TrackId(ether_core::protocol::model::Ulid(id)),
                peak,
                rms,
                clipped,
            });
        }
        Ok(Self {
            playhead,
            meters,
            event_overflow: flags & 4 != 0,
            underruns,
            blocks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::ParamTarget;
    use ether_core::graph::TrackDesc;
    use ether_core::protocol::model::{Beats, TrackKind};

    fn track_id(n: u128) -> TrackId {
        TrackId(ether_core::protocol::model::Ulid(n))
    }

    #[test]
    fn json_messages_roundtrip() {
        let key = NodeKey {
            index: 3,
            generation: 7,
        };
        let graph = RenderGraphDesc {
            version: 4,
            loop_enabled: true,
            loop_start: 1.0 / 3.0,
            loop_end: 8.0,
            tracks: vec![TrackDesc {
                id: track_id(42),
                kind: TrackKind::Master,
                chain: vec![],
                output: None,
                group: None,
                sends: vec![],
                volume: 0.5,
                pan: 0.0,
                mute: false,
                solo: false,
                audio_input: None,
                monitor: false,
                armed: false,
                clips: vec![],
                automation: vec![],
            }],
            ..Default::default()
        };
        let msgs = vec![
            EngineMsg::CreateBuiltin {
                key,
                device: BuiltinDevice::Sampler { sample: None },
                params: vec![(ParamId(1), 0.25)],
            },
            EngineMsg::DestroyNode { key },
            EngineMsg::Publish {
                graph: Box::new(graph),
            },
            EngineMsg::SetParam {
                change: ParamChange {
                    target: ParamTarget::Node {
                        node: key,
                        param: ParamId(2),
                    },
                    value: 0.1,
                },
            },
            EngineMsg::Transport {
                control: TransportControl::Locate {
                    position: Beats(3.5),
                },
            },
        ];
        for m in msgs {
            assert_eq!(EngineMsg::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn media_roundtrip_is_exact() {
        let media: MediaId = MediaId(ether_core::protocol::model::Ulid(0xABCDEF));
        let audio = Arc::new(DecodedAudio {
            sample_rate: 44_100,
            channels: vec![vec![0.1, -0.5, 1.0e-30], vec![f32::MAX, 0.0, -0.0]],
        });
        let msg = EngineMsg::LoadMedia {
            media,
            audio: audio.clone(),
        };
        let bytes = msg.encode();
        assert_eq!(bytes.len(), 1 + 1 + 26 + 4 + 2 + 4 + 6 * 4);
        assert_eq!(EngineMsg::decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn empty_media_keeps_channel_count() {
        let media = MediaId(ether_core::protocol::model::Ulid(1));
        let msg = EngineMsg::LoadMedia {
            media,
            audio: Arc::new(DecodedAudio {
                sample_rate: 48_000,
                channels: vec![vec![], vec![]],
            }),
        };
        assert_eq!(EngineMsg::decode(&msg.encode()).unwrap(), msg);
    }

    #[test]
    fn bad_messages_are_errors() {
        assert_eq!(EngineMsg::decode(&[]), Err(DecodeError::Empty));
        assert_eq!(EngineMsg::decode(b"X"), Err(DecodeError::Tag(b'X')));
        assert!(matches!(
            EngineMsg::decode(b"J{nope"),
            Err(DecodeError::Json(_))
        ));
        assert_eq!(
            EngineMsg::decode(&[TAG_MEDIA, 200]),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn report_roundtrip_without_realloc() {
        let report = EngineReport {
            playhead: PlayheadState {
                playing: true,
                recording: false,
                position: Beats(12.25),
                seconds: 6.125,
                bpm: 120.0,
                sample_time: 294_000,
            },
            meters: vec![TrackMeter {
                track: track_id(u128::MAX - 5),
                peak: [0.5, 0.25],
                rms: [0.1, 0.05],
                clipped: true,
            }],
            event_overflow: true,
            underruns: 3,
            blocks: 999,
        };
        let mut buf = Vec::with_capacity(EngineReport::encoded_len(1));
        let cap = buf.capacity();
        report.encode_into(&mut buf);
        assert_eq!(buf.len(), EngineReport::encoded_len(1));
        assert_eq!(buf.capacity(), cap);
        assert_eq!(EngineReport::decode(&buf).unwrap(), report);
    }
}
