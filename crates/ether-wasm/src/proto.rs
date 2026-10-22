//! Messages between the controller Worker and the AudioWorklet (over [`crate::ring`]).
//!
//! - Worker → Worklet: [`EngineMsg`]. Plain-data engine calls (the `EngineBridge` surface),
//!   each encoded into one or more ring frames and decoded on the Worklet as [`Frame`]s.
//!   Graph snapshots (`Publish`) use the binary [`BinaryCodec`] (tag `b'G'`): the Worklet
//!   decodes them on the audio thread, so no JSON parsing there. Decoded media is raw `f32`
//!   split into bounded chunks (`MediaBegin`, `MediaChunk`s, `MediaEnd`; see
//!   [`MediaAssembler`]), so no single frame makes the audio thread convert or buffer a whole
//!   file. A convolution reverb's IR prepared on the Worker travels the same way:
//!   [`Frame::IrBegin`] then [`Frame::IrChunk`]s of its flat `f32` stream
//!   ([`encode_prepared_ir`]; assembled by `ether_devices::fx_space::ConvolverAssembly`).
//!   The remaining small messages are JSON (tag `b'J'`).
//! - Worklet → Worker: [`EngineReport`] (playhead, max-held meters, diagnostics; compact
//!   binary encoded into a reused buffer so the audio thread doesn't allocate),
//!   [`REPORT_ANALYSIS`] device analysis frames (`fx-analysis`, [`encode_analysis_into`])
//!   [`REPORT_LATENCY`] node latency changes ([`crate::latency`]) and
//!   [`REPORT_ERROR`] text messages (compile errors etc.).

use std::collections::BTreeMap;
use std::sync::Arc;

use ether_core::analysis::{ANALYSIS_MAX_VALUES, AnalysisFrame, AnalysisKind};
use ether_core::codec::{BinaryCodec, CodecError, GraphCodec};
use ether_core::protocol::meters::TrackMeter;
use ether_core::protocol::model::{
    BuiltinDevice, IrSource, MediaId, MusicalScale, ParamId, TrackId, Ulid,
};
use ether_core::{NodeKey, ParamChange, PlayheadState, RenderGraphDesc, TransportControl};
use ether_devices::fx_space::{IrHeader, PreparedIr, Shaping};
use ether_media::DecodedAudio;
use serde::{Deserialize, Serialize};

const TAG_JSON: u8 = b'J';
/// `[G][BinaryCodec encoding]`: a `Publish`.
const TAG_GRAPH: u8 = b'G';
/// Initial capacity of a graph frame (Worker side; grows as needed).
const GRAPH_FRAME_RESERVE: usize = 64 * 1024;
const TAG_MEDIA_BEGIN: u8 = b'B';
const TAG_MEDIA_CHUNK: u8 = b'C';
const TAG_MEDIA_END: u8 = b'D';
/// `keyboard-midi`: `[M][status][data1][data2]`, an [`EngineMsg::LiveMidi`] (binary: decoded
/// on the audio thread without parsing or allocating).
const TAG_LIVE_MIDI: u8 = b'M';
/// `[N][track ulid, 16 bytes LE][status][data1][data2]`, an
/// [`EngineMsg::PreviewNote`] (binary like `LiveMidi`).
const TAG_PREVIEW_NOTE: u8 = b'N';
/// One channel of a streamed chunk (see [`encode_stream_chunk`]).
const TAG_STREAM_CHUNK: u8 = b'S';
/// A prepared convolution IR for a live node (see [`encode_prepared_ir`]).
const TAG_IR_BEGIN: u8 = b'I';
const TAG_IR_CHUNK: u8 = b'K';
/// `[P][count u32][count x i16 LE]`: interleaved stereo frames of the render preview
/// ([`EngineMsg::RenderPreviewChunk`]).
const TAG_RENDER_PREVIEW: u8 = b'P';
/// [`Frame::StreamChunk`] flags: first frame of the chunk (claims the slot), last one
/// (publishes it).
pub const STREAM_BEGIN: u8 = 1;
pub const STREAM_END: u8 = 2;
/// Samples per media chunk frame (64 KiB of `f32`).
pub const MEDIA_CHUNK_SAMPLES: usize = 16 * 1024;
/// `media-preview`: the reserved media id preview audio is shipped under (the ordinary
/// `LoadMedia` chunk path), then played by [`EngineMsg::Preview`]. Never a project media.
pub const PREVIEW_MEDIA: MediaId = MediaId(Ulid(u128::MAX));

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
    /// Suspend or resume the engine's processing while a project loads
    /// (`EngineBridge::set_suspended`).
    Suspend {
        suspended: bool,
    },
    /// `media-preview`: play the audio loaded as `media` (always [`PREVIEW_MEDIA`]) on the
    /// preview voice as preview `id` at `gain`; `media: None` stops the preview.
    Preview {
        id: u64,
        media: Option<MediaId>,
        gain: f32,
    },
    /// v0.2 (`midi-fx`): the resolved scale for a live node (`Node::set_data` with a
    /// `Box<MusicalScale>`; `EngineBridge::set_node_scale`).
    NodeScale {
        key: NodeKey,
        scale: MusicalScale,
    },
    /// v0.2 analysis channel: start/stop collecting a node's analysis frames
    /// (`EngineHandle::watch_analysis`; the Worklet reports them as [`REPORT_ANALYSIS`]).
    WatchAnalysis {
        key: NodeKey,
        on: bool,
    },
    /// v0.3: register a streamed media: allocate its chunk cache
    /// (`slots` chunks of `CHUNK_FRAMES` frames, `frames` long at the engine rate) and add
    /// it as the media's source. Chunks follow as [`Frame::StreamChunk`]s
    /// ([`crate::media_stream`]).
    StreamOpen {
        media: MediaId,
        channels: u16,
        frames: u64,
        slots: u32,
    },
    /// v0.3 (`fx-space`): a data-only change of a live built-in (sampler slices,
    /// multisampler zones, convolution reverb IR; `EngineBridge::update_builtin`). The
    /// Worklet builds the node data from `device` and its sources and hands it to the node
    /// (`Node::set_data`), so the change doesn't re-create the node (no click).
    UpdateBuiltin {
        key: NodeKey,
        device: BuiltinDevice,
    },
    /// `keyboard-midi`: one short live MIDI message (`EngineBridge::live_midi`), queued to
    /// the engine's live MIDI ring like hardware input (played from the next block).
    LiveMidi {
        data: [u8; 3],
    },
    /// One short MIDI message for one track's instrument
    /// (`EngineBridge::preview_note`, piano-roll note preview).
    PreviewNote {
        track: TrackId,
        data: [u8; 3],
    },
    /// A render's preview starts (`EngineBridge::begin_render_preview`): the Worklet
    /// allocates its writer and hands the buffer to the engine.
    RenderPreviewBegin {
        spec: ether_core::render_preview::PreviewSpec,
    },
    /// The next rendered frames of the preview (interleaved 16-bit stereo; binary).
    RenderPreviewChunk {
        samples: Vec<i16>,
    },
    /// The render ended: the engine drops its preview.
    RenderPreviewEnd,
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
    SetParam {
        change: ParamChange,
    },
    Transport {
        control: TransportControl,
    },
    Suspend {
        suspended: bool,
    },
    Preview {
        media: Option<MediaId>,
        gain: f32,
        id: u64,
    },
    NodeScale {
        key: NodeKey,
        scale: MusicalScale,
    },
    WatchAnalysis {
        key: NodeKey,
        on: bool,
    },
    StreamOpen {
        media: MediaId,
        channels: u16,
        frames: u64,
        slots: u32,
    },
    UpdateBuiltin {
        key: NodeKey,
        device: BuiltinDevice,
    },
    RenderPreviewBegin {
        spec: ether_core::render_preview::PreviewSpec,
    },
    RenderPreviewEnd,
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
    #[error("graph: {0}")]
    Graph(#[from] CodecError),
}

impl EngineMsg {
    /// Encode into ring frames: one, or `MediaBegin` + chunks + `MediaEnd` for media.
    pub fn encode(&self) -> Vec<Vec<u8>> {
        match self {
            EngineMsg::LoadMedia { media, audio } => return encode_media(*media, audio),
            EngineMsg::LiveMidi { data } => {
                return vec![vec![TAG_LIVE_MIDI, data[0], data[1], data[2]]];
            }
            EngineMsg::PreviewNote { track, data } => {
                let mut out = Vec::with_capacity(20);
                out.push(TAG_PREVIEW_NOTE);
                out.extend_from_slice(&track.0.0.to_le_bytes());
                out.extend_from_slice(data);
                return vec![out];
            }
            EngineMsg::Publish { graph } => {
                let mut out = Vec::with_capacity(GRAPH_FRAME_RESERVE);
                out.push(TAG_GRAPH);
                BinaryCodec.encode(graph, &mut out);
                return vec![out];
            }
            EngineMsg::RenderPreviewChunk { samples } => {
                let mut out = Vec::with_capacity(5 + samples.len() * 2);
                out.push(TAG_RENDER_PREVIEW);
                out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
                for s in samples {
                    out.extend_from_slice(&s.to_le_bytes());
                }
                return vec![out];
            }
            _ => {}
        }
        let json = match self.clone() {
            EngineMsg::LoadMedia { .. }
            | EngineMsg::Publish { .. }
            | EngineMsg::LiveMidi { .. }
            | EngineMsg::RenderPreviewChunk { .. }
            | EngineMsg::PreviewNote { .. } => unreachable!(),
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
            EngineMsg::SetParam { change } => JsonMsg::SetParam { change },
            EngineMsg::Transport { control } => JsonMsg::Transport { control },
            EngineMsg::Suspend { suspended } => JsonMsg::Suspend { suspended },
            EngineMsg::Preview { id, media, gain } => JsonMsg::Preview { media, gain, id },
            EngineMsg::NodeScale { key, scale } => JsonMsg::NodeScale { key, scale },
            EngineMsg::WatchAnalysis { key, on } => JsonMsg::WatchAnalysis { key, on },
            EngineMsg::StreamOpen {
                media,
                channels,
                frames,
                slots,
            } => JsonMsg::StreamOpen {
                media,
                channels,
                frames,
                slots,
            },
            EngineMsg::UpdateBuiltin { key, device } => JsonMsg::UpdateBuiltin { key, device },
            EngineMsg::RenderPreviewBegin { spec } => JsonMsg::RenderPreviewBegin { spec },
            EngineMsg::RenderPreviewEnd => JsonMsg::RenderPreviewEnd,
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
    /// Channel `channel` of chunk `chunk`, for cache slot `slot`.
    StreamChunk {
        media: MediaId,
        slot: u32,
        chunk: u64,
        channel: u16,
        /// [`STREAM_BEGIN`] | [`STREAM_END`].
        flags: u8,
        /// `CHUNK_FRAMES` `f32` LE samples.
        samples: &'a [u8],
    },
    /// A prepared IR for node `key` follows (a convolver to allocate for `header`,
    /// built at `sample_rate` from `source`); its stream arrives as [`Frame::IrChunk`]s.
    IrBegin {
        key: NodeKey,
        sample_rate: u32,
        header: IrHeader,
        source: IrSource,
    },
    /// The next piece of node `key`'s prepared IR stream: `offset` values into `segment`.
    IrChunk {
        key: NodeKey,
        segment: u8,
        offset: u32,
        /// `f32` LE values.
        values: &'a [u8],
    },
    /// The next frames of the render preview: interleaved stereo `i16` LE samples.
    RenderPreviewChunk {
        samples: &'a [u8],
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
            TAG_LIVE_MIDI => Ok(Frame::Msg(EngineMsg::LiveMidi {
                data: [c.u8()?, c.u8()?, c.u8()?],
            })),
            TAG_PREVIEW_NOTE => Ok(Frame::Msg(EngineMsg::PreviewNote {
                track: c.track()?,
                data: [c.u8()?, c.u8()?, c.u8()?],
            })),
            TAG_STREAM_CHUNK => {
                let media = c.media()?;
                let slot = c.u32()?;
                let chunk = c.u64()?;
                let channel = c.u16()?;
                let flags = c.u8()?;
                let count = c.u32()? as usize;
                let samples = c.take(count * 4)?;
                Ok(Frame::StreamChunk {
                    media,
                    slot,
                    chunk,
                    channel,
                    flags,
                    samples,
                })
            }
            TAG_IR_BEGIN => {
                let key = c.key()?;
                let sample_rate = c.u32()?;
                let channels = c.u8()?;
                let frames = c.u32()?;
                let shaping = Shaping {
                    decay: c.u16()?,
                    size: c.u16()?,
                    reverse: c.u8()? != 0,
                };
                let source =
                    serde_json::from_slice(c.0).map_err(|e| DecodeError::Json(e.to_string()))?;
                Ok(Frame::IrBegin {
                    key,
                    sample_rate,
                    header: IrHeader {
                        channels,
                        frames,
                        shaping,
                    },
                    source,
                })
            }
            TAG_IR_CHUNK => {
                let key = c.key()?;
                let segment = c.u8()?;
                let offset = c.u32()?;
                let count = c.u32()? as usize;
                let values = c.take(count * 4)?;
                Ok(Frame::IrChunk {
                    key,
                    segment,
                    offset,
                    values,
                })
            }
            TAG_RENDER_PREVIEW => {
                let count = c.u32()? as usize;
                Ok(Frame::RenderPreviewChunk {
                    samples: c.take(count * 2)?,
                })
            }
            TAG_GRAPH => Ok(Frame::Msg(EngineMsg::Publish {
                graph: Box::new(BinaryCodec.decode(rest)?),
            })),
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
                    JsonMsg::SetParam { change } => EngineMsg::SetParam { change },
                    JsonMsg::Transport { control } => EngineMsg::Transport { control },
                    JsonMsg::Suspend { suspended } => EngineMsg::Suspend { suspended },
                    JsonMsg::Preview { media, gain, id } => EngineMsg::Preview { id, media, gain },
                    JsonMsg::NodeScale { key, scale } => EngineMsg::NodeScale { key, scale },
                    JsonMsg::WatchAnalysis { key, on } => EngineMsg::WatchAnalysis { key, on },
                    JsonMsg::StreamOpen {
                        media,
                        channels,
                        frames,
                        slots,
                    } => EngineMsg::StreamOpen {
                        media,
                        channels,
                        frames,
                        slots,
                    },
                    JsonMsg::UpdateBuiltin { key, device } => {
                        EngineMsg::UpdateBuiltin { key, device }
                    }
                    JsonMsg::RenderPreviewBegin { spec } => EngineMsg::RenderPreviewBegin { spec },
                    JsonMsg::RenderPreviewEnd => EngineMsg::RenderPreviewEnd,
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

// [S][media u128][slot u32][chunk u64][channel u16][flags u8][count u32][count x f32 LE]
/// One frame per channel of a decoded chunk (`channels[c]`).
pub fn encode_stream_chunk(
    media: MediaId,
    slot: u32,
    chunk: u64,
    channels: &[Vec<f32>],
) -> Vec<Vec<u8>> {
    let id = media.0.0.to_le_bytes();
    let n = channels.len();
    channels
        .iter()
        .enumerate()
        .map(|(c, data)| {
            let mut flags = 0;
            if c == 0 {
                flags |= STREAM_BEGIN;
            }
            if c + 1 == n {
                flags |= STREAM_END;
            }
            let mut f = Vec::with_capacity(1 + 16 + 4 + 8 + 2 + 1 + 4 + data.len() * 4);
            f.push(TAG_STREAM_CHUNK);
            f.extend_from_slice(&id);
            f.extend_from_slice(&slot.to_le_bytes());
            f.extend_from_slice(&chunk.to_le_bytes());
            f.extend_from_slice(&(c as u16).to_le_bytes());
            f.push(flags);
            f.extend_from_slice(&(data.len() as u32).to_le_bytes());
            for s in data {
                f.extend_from_slice(&s.to_le_bytes());
            }
            f
        })
        .collect()
}

// Begin: [I][key u32 u32][sample_rate u32][channels u8][frames u32][decay u16][size u16]
//        [reverse u8][source JSON]
// Chunk: [K][key u32 u32][segment u8][offset u32][count u32][count x f32 LE]
/// A convolution IR prepared on the Worker, for the Worklet's live node `key`: one
/// begin frame, then the segments of its stream in order, each split into frames of at most
/// [`MEDIA_CHUNK_SAMPLES`] values (even counts, so spectra stay whole `re, im` pairs).
pub fn encode_prepared_ir(
    key: NodeKey,
    source: &IrSource,
    sample_rate: u32,
    ir: &PreparedIr,
) -> Vec<Vec<u8>> {
    let h = ir.header();
    let k = |f: &mut Vec<u8>| {
        f.extend_from_slice(&key.index.to_le_bytes());
        f.extend_from_slice(&key.generation.to_le_bytes());
    };
    let mut begin = vec![TAG_IR_BEGIN];
    k(&mut begin);
    begin.extend_from_slice(&sample_rate.to_le_bytes());
    begin.push(h.channels);
    begin.extend_from_slice(&h.frames.to_le_bytes());
    begin.extend_from_slice(&h.shaping.decay.to_le_bytes());
    begin.extend_from_slice(&h.shaping.size.to_le_bytes());
    begin.push(u8::from(h.shaping.reverse));
    serde_json::to_writer(&mut begin, source).expect("IR sources serialize");
    let mut out = vec![begin];
    for seg in 0..ether_devices::fx_space::prepared::SEGMENTS {
        let values = ir.segment(seg);
        debug_assert_eq!(values.len(), h.segment_len(seg));
        for (i, piece) in values.chunks(MEDIA_CHUNK_SAMPLES).enumerate() {
            let mut f = Vec::with_capacity(1 + 8 + 1 + 8 + piece.len() * 4);
            f.push(TAG_IR_CHUNK);
            k(&mut f);
            f.push(seg as u8);
            f.extend_from_slice(&((i * MEDIA_CHUNK_SAMPLES) as u32).to_le_bytes());
            f.extend_from_slice(&(piece.len() as u32).to_le_bytes());
            for v in piece {
                f.extend_from_slice(&v.to_le_bytes());
            }
            out.push(f);
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
    fn track(&mut self) -> Result<TrackId, DecodeError> {
        let id = u128::from_le_bytes(self.take(16)?.try_into().unwrap());
        Ok(TrackId(Ulid(id)))
    }
    fn key(&mut self) -> Result<NodeKey, DecodeError> {
        Ok(NodeKey {
            index: self.u32()?,
            generation: self.u32()?,
        })
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
/// Tag of an analysis frame (Worklet → Worker; `fx-analysis`).
pub const REPORT_ANALYSIS: u8 = b'A';
/// Tag of a node latency report (Worklet → Worker,
/// [`crate::latency::LatencyReport`]).
pub const REPORT_LATENCY: u8 = b'L';

// [A][node index u32][node generation u32][kind u8][len u16][len x f32 LE]; the node key is
// the Worker's virtual key.
const ANALYSIS_HEADER: usize = 1 + 4 + 4 + 1 + 2;
/// Bytes of the largest analysis report (pre-size the Worklet's buffer with it).
pub const ANALYSIS_REPORT_BYTES: usize = ANALYSIS_HEADER + 4 * ANALYSIS_MAX_VALUES;

fn analysis_kind_tag(kind: AnalysisKind) -> u8 {
    match kind {
        AnalysisKind::Spectrum => 0,
        AnalysisKind::Tuner => 1,
        AnalysisKind::Levels => 2,
        AnalysisKind::Modulation => 3,
        AnalysisKind::SpectrumPre => 4,
    }
}

/// **RT** (no allocation when `out` has [`ANALYSIS_REPORT_BYTES`] of capacity). Encode
/// `frame`, produced by the node the Worker knows as `node`, into `out` (cleared first).
pub fn encode_analysis_into(frame: &AnalysisFrame, node: NodeKey, out: &mut Vec<u8>) {
    out.clear();
    out.push(REPORT_ANALYSIS);
    out.extend_from_slice(&node.index.to_le_bytes());
    out.extend_from_slice(&node.generation.to_le_bytes());
    out.push(analysis_kind_tag(frame.kind));
    let values = frame.values();
    out.extend_from_slice(&(values.len() as u16).to_le_bytes());
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Decode a [`REPORT_ANALYSIS`] message (Worker side).
pub fn decode_analysis(bytes: &[u8]) -> Result<AnalysisFrame, DecodeError> {
    let mut c = Cursor(bytes);
    if c.u8()? != REPORT_ANALYSIS {
        return Err(DecodeError::Tag(bytes[0]));
    }
    let mut frame = AnalysisFrame::EMPTY;
    frame.node = NodeKey {
        index: c.u32()?,
        generation: c.u32()?,
    };
    let kind = match c.u8()? {
        0 => AnalysisKind::Spectrum,
        1 => AnalysisKind::Tuner,
        2 => AnalysisKind::Levels,
        3 => AnalysisKind::Modulation,
        4 => AnalysisKind::SpectrumPre,
        t => return Err(DecodeError::Tag(t)),
    };
    frame.begin(kind);
    let n = c.u16()? as usize;
    if n > ANALYSIS_MAX_VALUES {
        return Err(DecodeError::Truncated);
    }
    for _ in 0..n {
        frame.push(c.f32()?);
    }
    Ok(frame)
}

/// Engine outputs since the previous report, Worklet → Worker.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineReport {
    pub playhead: PlayheadState,
    pub meters: Vec<TrackMeter>,
    pub event_overflow: bool,
    pub underruns: u32,
    /// Blocks rendered since the Worklet started (diagnostics / liveness).
    pub blocks: u64,
    /// `media-preview`: id of the latest preview that played to its natural end since the
    /// last delivered report (`EngineOutputs::preview_ended`).
    pub preview_ended: Option<u64>,
}

// [R][flags u8][position f64][seconds f64][bpm f64][sample_time u64][blocks u64]
// [preview_ended u64, 0 = none][underruns u32][count u16]
// then per meter: [track u128 LE][peak f32 x2][rms f32 x2][clipped u8]
const METER_BYTES: usize = 16 + 16 + 1;
const REPORT_FIXED: usize = 1 + 1 + 8 * 6 + 4 + 2;

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
        out.extend_from_slice(&self.preview_ended.unwrap_or(0).to_le_bytes());
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
        let preview_ended = Some(c.u64()?).filter(|&id| id != 0);
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
            preview_ended,
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
                Frame::StreamChunk { .. }
                | Frame::IrBegin { .. }
                | Frame::IrChunk { .. }
                | Frame::RenderPreviewChunk { .. } => {
                    panic!("unexpected frame")
                }
            }
        }
        done
    }

    #[test]
    fn stream_messages_roundtrip() {
        let media = MediaId(Ulid(0x5EED));
        let open = EngineMsg::StreamOpen {
            media,
            channels: 2,
            frames: 48_000 * 600,
            slots: 26,
        };
        assert_eq!(decode_one(&open), open);
        let chans = vec![vec![0.25f32; 8], vec![-0.5f32; 8]];
        let frames = encode_stream_chunk(media, 3, 77, &chans);
        assert_eq!(frames.len(), 2);
        for (c, f) in frames.iter().enumerate() {
            match Frame::decode(f).unwrap() {
                Frame::StreamChunk {
                    media: m,
                    slot,
                    chunk,
                    channel,
                    flags,
                    samples,
                } => {
                    assert_eq!((m, slot, chunk, channel as usize), (media, 3, 77, c));
                    let want = if c == 0 { STREAM_BEGIN } else { STREAM_END };
                    assert_eq!(flags, want);
                    assert_eq!(samples.len(), 8 * 4);
                    assert_eq!(&samples[..4], &chans[c][0].to_le_bytes());
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn messages_roundtrip() {
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
                modulation: Default::default(),
                vca: Default::default(),
                chain_racks: Default::default(),
                frozen: Default::default(),
                input_tap: Default::default(),
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
                midi_input: Default::default(),
                monitor: false,
                armed: false,
                clips: vec![],
                automation: vec![],
                racks: Vec::new(),
                expression: Default::default(),
                hw_io: Vec::new(),
                sidechain_taps: Vec::new(),
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
            EngineMsg::NodeScale {
                key,
                scale: MusicalScale {
                    root: 2,
                    kind: ether_core::protocol::model::ScaleKind::Dorian,
                },
            },
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
            EngineMsg::Suspend { suspended: true },
            EngineMsg::Preview {
                id: 7,
                media: Some(PREVIEW_MEDIA),
                gain: 0.5,
            },
            EngineMsg::Preview {
                id: 8,
                media: None,
                gain: 1.0,
            },
            EngineMsg::RenderPreviewBegin {
                spec: ether_core::render_preview::PreviewSpec {
                    rate: 48_000,
                    start_seconds: 1.5,
                    frames: 96_000,
                    tempo: vec![ether_core::tempo::TempoPointDesc {
                        beat: 0.0,
                        bpm: 97.0,
                        curve: Default::default(),
                    }],
                },
            },
            EngineMsg::RenderPreviewEnd,
        ];
        for m in msgs {
            assert_eq!(decode_one(&m), m);
        }
    }

    #[test]
    fn render_preview_chunks_are_binary() {
        let msg = EngineMsg::RenderPreviewChunk {
            samples: vec![1, -2, i16::MAX, i16::MIN],
        };
        let frame = msg.encode().remove(0);
        assert_eq!(frame.len(), 1 + 4 + 8);
        let Frame::RenderPreviewChunk { samples } = Frame::decode(&frame).unwrap() else {
            panic!("a preview chunk");
        };
        let back: Vec<i16> = samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        assert_eq!(back, [1, -2, i16::MAX, i16::MIN]);
    }

    #[test]
    fn graph_frames_are_binary_and_versioned() {
        let msg = EngineMsg::Publish {
            graph: Box::new(RenderGraphDesc {
                version: 9,
                ..Default::default()
            }),
        };
        let mut frame = msg.encode().remove(0);
        assert_eq!(frame[..2], [TAG_GRAPH, BinaryCodec::VERSION]);
        assert_eq!(decode_one(&msg), msg);
        frame[1] = BinaryCodec::VERSION + 1;
        assert_eq!(
            Frame::decode(&frame),
            Err(DecodeError::Graph(CodecError::Version(
                BinaryCodec::VERSION + 1
            )))
        );
        frame.truncate(5);
        frame[1] = BinaryCodec::VERSION;
        assert!(matches!(
            Frame::decode(&frame),
            Err(DecodeError::Graph(CodecError::Malformed(_)))
        ));
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
            preview_ended: Some(42),
        };
        let mut buf = Vec::with_capacity(EngineReport::encoded_len(1));
        let cap = buf.capacity();
        report.encode_into(&mut buf);
        assert_eq!(buf.len(), EngineReport::encoded_len(1));
        assert_eq!(buf.capacity(), cap);
        assert_eq!(EngineReport::decode(&buf).unwrap(), report);
    }

    #[test]
    fn analysis_frames_roundtrip_without_realloc() {
        let mut f = AnalysisFrame::EMPTY;
        f.begin(AnalysisKind::Tuner);
        for v in [440.0, 69.0, -3.5, 0.9, -12.0] {
            f.push(v);
        }
        let node = NodeKey {
            index: 7,
            generation: 1,
        };
        let mut buf = Vec::with_capacity(ANALYSIS_REPORT_BYTES);
        let cap = buf.capacity();
        encode_analysis_into(&f, node, &mut buf);
        assert_eq!(buf.capacity(), cap);
        let back = decode_analysis(&buf).unwrap();
        assert_eq!(
            (back.node, back.kind, back.values()),
            (node, f.kind, f.values())
        );
        // A full frame fits the pre-sized buffer.
        f.begin(AnalysisKind::Spectrum);
        while f.push(-60.0) {}
        encode_analysis_into(&f, node, &mut buf);
        assert_eq!((buf.len(), buf.capacity()), (ANALYSIS_REPORT_BYTES, cap));
        assert_eq!(
            decode_analysis(&buf).unwrap().values().len(),
            ANALYSIS_MAX_VALUES
        );
        assert_eq!(
            decode_analysis(&buf[..20]).err(),
            Some(DecodeError::Truncated)
        );
        // The watch message round-trips.
        let msg = EngineMsg::WatchAnalysis {
            key: node,
            on: true,
        };
        let frames = msg.encode();
        assert_eq!(Frame::decode(&frames[0]).unwrap(), Frame::Msg(msg));
    }
}
