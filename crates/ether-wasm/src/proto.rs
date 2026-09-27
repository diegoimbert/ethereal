//! Messages between the controller Worker and the AudioWorklet (over [`crate::ring`]).
//!
//! - Worker → Worklet: [`EngineMsg`]. Plain-data engine calls (the `EngineBridge` surface),
//!   each encoded into one or more ring frames and decoded on the Worklet as [`Frame`]s.
//!   Everything is JSON (tag `b'J'`) except decoded media: raw `f32` split into bounded
//!   chunks (`MediaBegin`, `MediaChunk`s, `MediaEnd`; see [`MediaAssembler`]), so no single
//!   frame makes the audio thread convert or buffer a whole file.
//! - Worklet → Worker: [`EngineReport`] (playhead, max-held meters, diagnostics; compact
//!   binary encoded into a reused buffer so the audio thread doesn't allocate) and
//!   [`REPORT_ERROR`] text messages (compile errors etc.).

use std::collections::BTreeMap;
use std::sync::Arc;

use ether_core::protocol::meters::TrackMeter;
use ether_core::protocol::model::{BuiltinDevice, MediaId, ParamId, TrackId, Ulid};
use ether_core::{NodeKey, ParamChange, PlayheadState, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;
use serde::{Deserialize, Serialize};

const TAG_JSON: u8 = b'J';
const TAG_MEDIA_BEGIN: u8 = b'B';
const TAG_MEDIA_CHUNK: u8 = b'C';
const TAG_MEDIA_END: u8 = b'D';
/// Samples per media chunk frame (64 KiB of `f32`).
pub const MEDIA_CHUNK_SAMPLES: usize = 16 * 1024;

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
    /// Encode into ring frames: one, or `MediaBegin` + chunks + `MediaEnd` for media.
    pub fn encode(&self) -> Vec<Vec<u8>> {
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
        vec![out]
    }
}

/// One decoded ring frame (Worklet side). Media chunks borrow the frame bytes; the
/// [`MediaAssembler`] converts them straight into pre-allocated buffers.
#[derive(Debug, PartialEq)]
pub enum Frame<'a> {
    /// Any non-media [`EngineMsg`].
    Msg(EngineMsg),
    MediaBegin {
        media: MediaId,
        sample_rate: u32,
        channels: u16,
        frames: u32,
    },
    MediaChunk {
        media: MediaId,
        channel: u16,
        offset: u32,
        /// `f32` LE samples.
        samples: &'a [u8],
    },
    MediaEnd {
        media: MediaId,
    },
}

impl<'a> Frame<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        let (&tag, rest) = bytes.split_first().ok_or(DecodeError::Empty)?;
        let mut c = Cursor(rest);
        match tag {
            TAG_MEDIA_BEGIN => Ok(Frame::MediaBegin {
                media: c.media()?,
                sample_rate: c.u32()?,
                channels: c.u16()?,
                frames: c.u32()?,
            }),
            TAG_MEDIA_CHUNK => {
                let media = c.media()?;
                let channel = c.u16()?;
                let offset = c.u32()?;
                let count = c.u32()? as usize;
                let samples = c.take(count * 4)?;
                Ok(Frame::MediaChunk {
                    media,
                    channel,
                    offset,
                    samples,
                })
            }
            TAG_MEDIA_END => Ok(Frame::MediaEnd { media: c.media()? }),
            TAG_JSON => {
                let msg: JsonMsg =
                    serde_json::from_slice(rest).map_err(|e| DecodeError::Json(e.to_string()))?;
                Ok(Frame::Msg(match msg {
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
                }))
            }
            t => Err(DecodeError::Tag(t)),
        }
    }
}

// Begin: [B][media u128][sample_rate u32][channels u16][frames u32]
// Chunk: [C][media u128][channel u16][offset u32][count u32][count x f32 LE]
// End:   [D][media u128]
fn encode_media(media: MediaId, audio: &DecodedAudio) -> Vec<Vec<u8>> {
    let id = media.0.0.to_le_bytes();
    let frames = audio.frames();
    let mut out = Vec::new();
    let mut begin = vec![TAG_MEDIA_BEGIN];
    begin.extend_from_slice(&id);
    begin.extend_from_slice(&audio.sample_rate.to_le_bytes());
    begin.extend_from_slice(&(audio.channels.len() as u16).to_le_bytes());
    begin.extend_from_slice(&(frames as u32).to_le_bytes());
    out.push(begin);
    for (ch, data) in audio.channels.iter().enumerate() {
        for (i, chunk) in data[..frames].chunks(MEDIA_CHUNK_SAMPLES).enumerate() {
            let mut f = Vec::with_capacity(1 + 16 + 10 + chunk.len() * 4);
            f.push(TAG_MEDIA_CHUNK);
            f.extend_from_slice(&id);
            f.extend_from_slice(&(ch as u16).to_le_bytes());
            f.extend_from_slice(&((i * MEDIA_CHUNK_SAMPLES) as u32).to_le_bytes());
            f.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            for s in chunk {
                f.extend_from_slice(&s.to_le_bytes());
            }
            out.push(f);
        }
    }
    let mut end = vec![TAG_MEDIA_END];
    end.extend_from_slice(&id);
    out.push(end);
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
    fn media(&mut self) -> Result<MediaId, DecodeError> {
        let id = u128::from_le_bytes(self.take(16)?.try_into().unwrap());
        Ok(MediaId(Ulid(id)))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_bits(self.u64()?))
    }
}

/// Reassembles chunked media on the Worklet. [`Self::begin`] allocates the planar buffers
/// once (the only allocation of a load); [`Self::chunk`] converts at most
/// [`MEDIA_CHUNK_SAMPLES`] samples straight into them; [`Self::end`] hands out the audio.
#[derive(Default)]
pub struct MediaAssembler {
    pending: BTreeMap<MediaId, DecodedAudio>,
}

impl MediaAssembler {
    pub fn begin(&mut self, media: MediaId, sample_rate: u32, channels: u16, frames: u32) {
        self.pending.insert(
            media,
            DecodedAudio {
                sample_rate,
                channels: (0..channels).map(|_| vec![0.0; frames as usize]).collect(),
            },
        );
    }

    pub fn chunk(
        &mut self,
        media: MediaId,
        channel: u16,
        offset: u32,
        samples: &[u8],
    ) -> Result<(), String> {
        let audio = self
            .pending
            .get_mut(&media)
            .ok_or_else(|| format!("media chunk for {media} without begin"))?;
        let data = audio
            .channels
            .get_mut(channel as usize)
            .ok_or_else(|| format!("media {media}: bad channel {channel}"))?;
        let offset = offset as usize;
        let dst = data
            .get_mut(offset..offset + samples.len() / 4)
            .ok_or_else(|| format!("media {media}: chunk out of range"))?;
        for (d, b) in dst.iter_mut().zip(samples.as_chunks::<4>().0) {
            *d = f32::from_le_bytes(*b);
        }
        Ok(())
    }

    /// The finished audio (`None` if the load was cancelled or never begun).
    pub fn end(&mut self, media: MediaId) -> Option<Arc<DecodedAudio>> {
        self.pending.remove(&media).map(Arc::new)
    }

    /// Drop a partially received load (the media was unloaded meanwhile).
    pub fn cancel(&mut self, media: MediaId) {
        self.pending.remove(&media);
    }
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
                track: TrackId(Ulid(id)),
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
        TrackId(Ulid(n))
    }

    fn decode_one(m: &EngineMsg) -> EngineMsg {
        let frames = m.encode();
        assert_eq!(frames.len(), 1);
        match Frame::decode(&frames[0]).unwrap() {
            Frame::Msg(m) => m,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Run frames through a `MediaAssembler` the way the Worklet does.
    fn assemble(frames: &[Vec<u8>]) -> Option<(MediaId, Arc<DecodedAudio>)> {
        let mut asm = MediaAssembler::default();
        let mut done = None;
        for f in frames {
            match Frame::decode(f).unwrap() {
                Frame::MediaBegin {
                    media,
                    sample_rate,
                    channels,
                    frames,
                } => asm.begin(media, sample_rate, channels, frames),
                Frame::MediaChunk {
                    media,
                    channel,
                    offset,
                    samples,
                } => asm.chunk(media, channel, offset, samples).unwrap(),
                Frame::MediaEnd { media } => done = asm.end(media).map(|a| (media, a)),
                Frame::Msg(m) => panic!("unexpected {m:?}"),
            }
        }
        done
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
                racks: Vec::new(),
            }],
            ..Default::default()
        };
        let msgs = vec![
            EngineMsg::CreateBuiltin {
                key,
                device: BuiltinDevice::Sampler {
                    sample: None,
                    slices: Default::default(),
                },
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
            assert_eq!(decode_one(&m), m);
        }
    }

    #[test]
    fn media_is_chunked_and_reassembles_exactly() {
        let media = MediaId(Ulid(0xABCDEF));
        let frames = MEDIA_CHUNK_SAMPLES * 2 + 7;
        let left: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.001).sin()).collect();
        let mut right = left.clone();
        right[0] = f32::MAX;
        right[frames - 1] = -0.0;
        let audio = Arc::new(DecodedAudio {
            sample_rate: 44_100,
            channels: vec![left, right],
        });
        let encoded = EngineMsg::LoadMedia {
            media,
            audio: audio.clone(),
        }
        .encode();
        // begin + 3 chunks per channel + end; no frame bigger than one chunk.
        assert_eq!(encoded.len(), 1 + 2 * 3 + 1);
        assert!(
            encoded
                .iter()
                .all(|f| f.len() <= 1 + 16 + 10 + MEDIA_CHUNK_SAMPLES * 4)
        );
        let (id, got) = assemble(&encoded).unwrap();
        assert_eq!(id, media);
        assert_eq!(*got, *audio);
    }

    #[test]
    fn empty_media_keeps_channel_count() {
        let media = MediaId(Ulid(1));
        let audio = Arc::new(DecodedAudio {
            sample_rate: 48_000,
            channels: vec![vec![], vec![]],
        });
        let encoded = EngineMsg::LoadMedia {
            media,
            audio: audio.clone(),
        }
        .encode();
        assert_eq!(*assemble(&encoded).unwrap().1, *audio);
    }

    #[test]
    fn cancelled_or_bad_media_chunks() {
        let media = MediaId(Ulid(5));
        let mut asm = MediaAssembler::default();
        assert!(
            asm.chunk(media, 0, 0, &[0; 4]).is_err(),
            "chunk before begin"
        );
        asm.begin(media, 48_000, 1, 2);
        assert!(asm.chunk(media, 1, 0, &[0; 4]).is_err(), "bad channel");
        assert!(asm.chunk(media, 0, 1, &[0; 8]).is_err(), "out of range");
        asm.cancel(media);
        assert!(asm.end(media).is_none());
    }

    #[test]
    fn bad_messages_are_errors() {
        assert_eq!(Frame::decode(&[]), Err(DecodeError::Empty));
        assert_eq!(Frame::decode(b"X"), Err(DecodeError::Tag(b'X')));
        assert!(matches!(
            Frame::decode(b"J{nope"),
            Err(DecodeError::Json(_))
        ));
        assert_eq!(
            Frame::decode(&[TAG_MEDIA_CHUNK, 200]),
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
