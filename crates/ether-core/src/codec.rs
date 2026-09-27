//! Render-graph snapshot codec (roadmap v2, owned by the `web-perf` node; see
//! `docs/ROADMAP.md`).
//!
//! The web host ships every published [`RenderGraphDesc`] from the controller Worker to the
//! AudioWorklet. [`BinaryCodec`] is the compact binary encoding behind this trait (it
//! replaced the JSON path in `ether-wasm/src/proto.rs`); both ends of a connection must use
//! the same codec. Contract:
//!
//! - `decode(encode(d)) == d` for every desc (exact `f64`s, ids, ordering).
//! - `encode` runs on the controller side (Worker; may allocate).
//! - **`decode` runs in the AudioWorklet** on the web: the worklet scope has no other
//!   thread, so decoding (and `compile_with`, which follows) runs on the audio rendering
//!   thread between two `process()` calls and allocates, exactly like today's JSON path.
//!   This is the one accepted exception to the RT rules on the web (native hosts decode
//!   nothing: they publish the desc directly). The codec's job is to make that step cheap
//!   and bounded: no string parsing, sizes known up front (reserve once), no per-field
//!   allocation beyond the desc's own `Vec`s. Natively, `decode` may run on any non-RT
//!   thread.
//! - Encoded data starts with a version byte; `decode` rejects unknown versions with
//!   [`CodecError::Version`] instead of misreading.

use ether_protocol::devices::ParamScale;
use ether_protocol::model::{
    AutomationTarget, ClipId, CurveShape, DeviceId, DrumPadId, FadeCurve, MediaId, MetronomeSound,
    ParamId, SendId, TempoCurve, TimeSignature, TrackId, TrackKind, Ulid, WarpMode,
};

use crate::graph::{
    AutomationDesc, ChainEntry, ClipContentDesc, ClipDesc, MetronomeDesc, NoteDesc, PadDesc,
    ParamMapping, RackDesc, RenderGraphDesc, ResolvedTarget, SendDesc, TrackDesc, WarpDesc,
};
use crate::node::NodeKey;
use crate::tempo::{TempoPointDesc, TimeSignatureDesc};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CodecError {
    #[error("unsupported snapshot encoding version {0}")]
    Version(u8),
    #[error("malformed snapshot: {0}")]
    Malformed(String),
}

/// Encodes/decodes render-graph snapshots for transfer between threads/instances.
pub trait GraphCodec {
    /// Append the encoding of `desc` to `out`.
    fn encode(&self, desc: &RenderGraphDesc, out: &mut Vec<u8>);
    fn decode(&self, bytes: &[u8]) -> Result<RenderGraphDesc, CodecError>;
}

/// The binary [`GraphCodec`] used between the web controller Worker and the AudioWorklet.
///
/// # Format (version [`BinaryCodec::VERSION`])
///
/// `[version u8][desc]`, then nothing (trailing bytes are rejected). Everything is
/// little-endian and fixed width, in struct field declaration order:
///
/// - `f64`/`f32` as their IEEE bits (`to_bits`), so the round trip is bit-exact (incl.
///   `-0.0`, subnormals, infinities and NaN payloads);
/// - ids as `u128` (the ULID), `ParamId` as `u32`, `NodeKey` as `index u32, generation u32`;
/// - `bool` as one byte `0`/`1` (anything else is malformed);
/// - `Option<T>` as a `0`/`1` byte, then `T` if present;
/// - enums as a `u8` variant tag (declaration order), then the variant's fields;
/// - `Vec<T>` as a `u32` element count, then the elements.
///
/// # Versioning
///
/// Any change to the layout above (a new field or variant in any desc type, a reordered
/// field, a wider integer) bumps [`BinaryCodec::VERSION`]; `decode` rejects every other
/// version with [`CodecError::Version`]. Both ends of a web session come from the same wasm
/// build, so no cross-version compatibility is needed, only detection. `encode` destructures
/// every desc type and matches every enum exhaustively, so adding a field or a variant fails
/// to compile here until the codec (and the version) is updated.
///
/// # Cost of `decode`
///
/// No parsing, no strings, no per-field allocation: one pass over the bytes, every `Vec`
/// allocated once with its exact length (read from its count), and counts are validated
/// against the bytes left before allocating (a corrupt count can't trigger a huge
/// allocation). On success the number of allocations is exactly the number of non-empty
/// `Vec`s in the desc (`crates/ether-core/tests/codec_alloc.rs` checks it); errors allocate
/// their message.
#[derive(Clone, Copy, Debug, Default)]
pub struct BinaryCodec;

impl BinaryCodec {
    /// Current format version (first byte of every encoding).
    pub const VERSION: u8 = 1;
}

impl GraphCodec for BinaryCodec {
    fn encode(&self, desc: &RenderGraphDesc, out: &mut Vec<u8>) {
        out.push(Self::VERSION);
        Writer(out).desc(desc);
    }

    fn decode(&self, bytes: &[u8]) -> Result<RenderGraphDesc, CodecError> {
        let (&version, rest) = bytes.split_first().ok_or_else(|| malformed("empty"))?;
        if version != Self::VERSION {
            return Err(CodecError::Version(version));
        }
        let mut r = Reader {
            bytes: rest,
            pos: 0,
        };
        let desc = r.desc()?;
        if r.pos != r.bytes.len() {
            return Err(CodecError::Malformed(format!(
                "{} trailing bytes",
                r.bytes.len() - r.pos
            )));
        }
        Ok(desc)
    }
}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn ulid(&mut self, v: Ulid) {
        self.0.extend_from_slice(&v.0.to_le_bytes());
    }
    fn opt<T>(&mut self, v: &Option<T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            None => self.u8(0),
            Some(v) => {
                self.u8(1);
                f(self, v);
            }
        }
    }
    fn vec<T>(&mut self, v: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.u32(u32::try_from(v.len()).expect("graph desc vec longer than u32::MAX"));
        for x in v {
            f(self, x);
        }
    }
    fn key(&mut self, k: NodeKey) {
        let NodeKey { index, generation } = k;
        self.u32(index);
        self.u32(generation);
    }

    fn desc(&mut self, d: &RenderGraphDesc) {
        let RenderGraphDesc {
            version,
            tempo,
            signatures,
            loop_enabled,
            loop_start,
            loop_end,
            metronome,
            click,
            tracks,
        } = d;
        self.u64(*version);
        self.vec(tempo, |w, p| {
            let TempoPointDesc { beat, bpm, curve } = *p;
            w.f64(beat);
            w.f64(bpm);
            w.u8(match curve {
                TempoCurve::Step => 0,
                TempoCurve::Linear => 1,
            });
        });
        self.vec(signatures, |w, s| {
            let TimeSignatureDesc {
                beat,
                signature:
                    TimeSignature {
                        numerator,
                        denominator,
                    },
            } = *s;
            w.f64(beat);
            w.u8(numerator);
            w.u8(denominator);
        });
        self.bool(*loop_enabled);
        self.f64(*loop_start);
        self.f64(*loop_end);
        self.bool(*metronome);
        let MetronomeDesc {
            volume,
            accent,
            sound,
            count_in_end,
        } = *click;
        self.f32(volume);
        self.bool(accent);
        self.u8(match sound {
            MetronomeSound::Classic => 0,
            MetronomeSound::Wood => 1,
            MetronomeSound::Beep => 2,
        });
        self.opt(&count_in_end, |w, v| w.f64(*v));
        self.vec(tracks, Self::track);
    }

    fn track(&mut self, t: &TrackDesc) {
        let TrackDesc {
            id,
            kind,
            chain,
            output,
            group,
            sends,
            volume,
            pan,
            mute,
            solo,
            audio_input,
            monitor,
            armed,
            clips,
            automation,
            racks,
        } = t;
        self.ulid(id.0);
        self.u8(match kind {
            TrackKind::Audio => 0,
            TrackKind::Midi => 1,
            TrackKind::Group => 2,
            TrackKind::Return => 3,
            TrackKind::Master => 4,
        });
        self.vec(chain, Self::chain_entry);
        self.opt(output, |w, v| w.ulid(v.0));
        self.opt(group, |w, v| w.ulid(v.0));
        self.vec(sends, |w, s| {
            let SendDesc {
                id,
                to,
                level,
                pre_fader,
            } = *s;
            w.ulid(id.0);
            w.ulid(to.0);
            w.f32(level);
            w.bool(pre_fader);
        });
        self.f32(*volume);
        self.f32(*pan);
        self.bool(*mute);
        self.bool(*solo);
        self.opt(audio_input, |w, &(first, count)| {
            w.u16(first);
            w.u16(count);
        });
        self.bool(*monitor);
        self.bool(*armed);
        self.vec(clips, Self::clip);
        self.vec(automation, Self::automation);
        self.vec(racks, |w, r| {
            let RackDesc { rack, pads } = r;
            w.key(*rack);
            w.vec(pads, |w, p| {
                let PadDesc {
                    pad,
                    note,
                    choke_group,
                    chain,
                    volume,
                    pan,
                    mute,
                } = p;
                w.ulid(pad.0);
                w.u8(*note);
                w.opt(choke_group, |w, v| w.u8(*v));
                w.vec(chain, Self::chain_entry);
                w.f32(*volume);
                w.f32(*pan);
                w.bool(*mute);
            });
        });
    }

    fn chain_entry(&mut self, e: &ChainEntry) {
        let ChainEntry {
            node,
            enabled,
            sidechain,
        } = *e;
        self.key(node);
        self.bool(enabled);
        self.opt(&sidechain, |w, v| w.ulid(v.0));
    }

    fn clip(&mut self, c: &ClipDesc) {
        let ClipDesc {
            id,
            start,
            length,
            offset,
            looping,
            muted,
            content,
            envelopes,
        } = c;
        self.ulid(id.0);
        self.f64(*start);
        self.f64(*length);
        self.f64(*offset);
        self.opt(looping, |w, &(a, b)| {
            w.f64(a);
            w.f64(b);
        });
        self.bool(*muted);
        match content {
            ClipContentDesc::Midi { notes } => {
                self.u8(0);
                self.vec(notes, |w, n| {
                    let NoteDesc {
                        start,
                        duration,
                        key,
                        velocity,
                        release_velocity,
                    } = *n;
                    w.f64(start);
                    w.f64(duration);
                    w.u8(key);
                    w.f32(velocity);
                    w.f32(release_velocity);
                });
            }
            ClipContentDesc::Audio {
                media,
                gain,
                transpose,
                fade_in,
                fade_out,
                warp,
                fade_in_curve,
                fade_out_curve,
                reversed,
            } => {
                self.u8(1);
                self.ulid(media.0);
                self.f32(*gain);
                self.f32(*transpose);
                self.f64(*fade_in);
                self.f64(*fade_out);
                self.opt(warp, |w, warp| {
                    let WarpDesc { mode, markers } = warp;
                    w.u8(match mode {
                        WarpMode::Repitch => 0,
                        WarpMode::Complex => 1,
                    });
                    w.vec(markers, |w, &(a, b)| {
                        w.f64(a);
                        w.f64(b);
                    });
                });
                self.fade(*fade_in_curve);
                self.fade(*fade_out_curve);
                self.bool(*reversed);
            }
        }
        self.vec(envelopes, Self::automation);
    }

    fn fade(&mut self, c: FadeCurve) {
        match c {
            FadeCurve::Linear => self.u8(0),
            FadeCurve::EqualPower => self.u8(1),
            FadeCurve::Curve { tension } => {
                self.u8(2);
                self.f32(tension);
            }
        }
    }

    fn automation(&mut self, a: &AutomationDesc) {
        let AutomationDesc {
            target,
            resolved,
            points,
            mapping,
        } = a;
        match *target {
            AutomationTarget::TrackVolume { track } => {
                self.u8(0);
                self.ulid(track.0);
            }
            AutomationTarget::TrackPan { track } => {
                self.u8(1);
                self.ulid(track.0);
            }
            AutomationTarget::SendLevel { send } => {
                self.u8(2);
                self.ulid(send.0);
            }
            AutomationTarget::DeviceParam { device, param } => {
                self.u8(3);
                self.ulid(device.0);
                self.u32(param.0);
            }
        }
        match *resolved {
            ResolvedTarget::TrackVolume => self.u8(0),
            ResolvedTarget::TrackPan => self.u8(1),
            ResolvedTarget::Send { send } => {
                self.u8(2);
                self.ulid(send.0);
            }
            ResolvedTarget::Node { node, param } => {
                self.u8(3);
                self.key(node);
                self.u32(param.0);
            }
        }
        self.vec(points, |w, &(time, value, curve)| {
            w.f64(time);
            w.f64(value);
            match curve {
                CurveShape::Linear => w.u8(0),
                CurveShape::Step => w.u8(1),
                CurveShape::Curve { tension } => {
                    w.u8(2);
                    w.f32(tension);
                }
            }
        });
        let ParamMapping {
            min,
            max,
            scale,
            steps,
        } = *mapping;
        self.f64(min);
        self.f64(max);
        match scale {
            ParamScale::Linear => self.u8(0),
            ParamScale::Log => self.u8(1),
            ParamScale::Power { exponent } => {
                self.u8(2);
                self.f64(exponent);
            }
            ParamScale::Fader => self.u8(3),
        }
        self.opt(&steps, |w, v| w.u32(*v));
    }
}

/// Smallest encoded size of one element per `Vec` element type (bounds a count by the bytes
/// left before allocating).
mod min_size {
    pub const TEMPO: usize = 8 + 8 + 1;
    pub const SIGNATURE: usize = 8 + 1 + 1;
    /// id, kind, 5 counts, 2 options, volume, pan, 4 bools, audio input option.
    pub const TRACK: usize = 16 + 1 + 5 * 4 + 2 + 4 + 4 + 4 + 1;
    pub const CHAIN: usize = 8 + 1 + 1;
    pub const SEND: usize = 16 + 16 + 4 + 1;
    /// id, start/length/offset, looping, muted, content tag + count, envelopes count.
    pub const CLIP: usize = 16 + 24 + 1 + 1 + 1 + 4 + 4;
    pub const NOTE: usize = 8 + 8 + 1 + 4 + 4;
    pub const MARKER: usize = 16;
    /// target (tag + id), resolved tag, points count, min/max, scale tag, steps option.
    pub const AUTOMATION: usize = 1 + 16 + 1 + 4 + 16 + 1 + 1;
    pub const POINT: usize = 8 + 8 + 1;
    pub const RACK: usize = 8 + 4;
    pub const PAD: usize = 16 + 1 + 1 + 4 + 4 + 4 + 1;
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

#[cold]
fn malformed(what: &str) -> CodecError {
    CodecError::Malformed(what.to_owned())
}

#[cold]
fn bad_tag(what: &str, tag: u8) -> CodecError {
    CodecError::Malformed(format!("bad {what} tag {tag}"))
}

impl Reader<'_> {
    #[inline]
    fn take<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        match self.bytes.get(self.pos..self.pos + N) {
            Some(b) => {
                self.pos += N;
                Ok(b.try_into().expect("N bytes"))
            }
            None => Err(malformed("truncated")),
        }
    }
    #[inline]
    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take::<1>()?[0])
    }
    #[inline]
    fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    #[inline]
    fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    #[inline]
    fn u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    #[inline]
    fn ulid(&mut self) -> Result<Ulid, CodecError> {
        Ok(Ulid(u128::from_le_bytes(self.take()?)))
    }
    #[inline]
    fn f32(&mut self) -> Result<f32, CodecError> {
        Ok(f32::from_bits(self.u32()?))
    }
    #[inline]
    fn f64(&mut self) -> Result<f64, CodecError> {
        Ok(f64::from_bits(self.u64()?))
    }
    #[inline]
    fn bool(&mut self) -> Result<bool, CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            t => Err(bad_tag("bool", t)),
        }
    }
    /// An enum tag in `0..count`.
    #[inline]
    fn tag(&mut self, count: u8, what: &str) -> Result<u8, CodecError> {
        match self.u8()? {
            t if t < count => Ok(t),
            t => Err(bad_tag(what, t)),
        }
    }
    #[inline]
    fn opt<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Option<T>, CodecError> {
        Ok(if self.bool()? { Some(f(self)?) } else { None })
    }
    /// A `Vec` of elements of at least `min` encoded bytes each, allocated once.
    #[inline]
    fn vec<T>(
        &mut self,
        min: usize,
        mut f: impl FnMut(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Vec<T>, CodecError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min) > self.bytes.len() - self.pos {
            return Err(malformed("count exceeds data"));
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(f(self)?);
        }
        Ok(v)
    }
    #[inline]
    fn key(&mut self) -> Result<NodeKey, CodecError> {
        Ok(NodeKey {
            index: self.u32()?,
            generation: self.u32()?,
        })
    }
    #[inline]
    fn track_id(&mut self) -> Result<TrackId, CodecError> {
        Ok(TrackId(self.ulid()?))
    }

    fn desc(&mut self) -> Result<RenderGraphDesc, CodecError> {
        Ok(RenderGraphDesc {
            version: self.u64()?,
            tempo: self.vec(min_size::TEMPO, |r| {
                Ok(TempoPointDesc {
                    beat: r.f64()?,
                    bpm: r.f64()?,
                    curve: match r.tag(2, "tempo curve")? {
                        0 => TempoCurve::Step,
                        _ => TempoCurve::Linear,
                    },
                })
            })?,
            signatures: self.vec(min_size::SIGNATURE, |r| {
                Ok(TimeSignatureDesc {
                    beat: r.f64()?,
                    signature: TimeSignature {
                        numerator: r.u8()?,
                        denominator: r.u8()?,
                    },
                })
            })?,
            loop_enabled: self.bool()?,
            loop_start: self.f64()?,
            loop_end: self.f64()?,
            metronome: self.bool()?,
            click: MetronomeDesc {
                volume: self.f32()?,
                accent: self.bool()?,
                sound: match self.tag(3, "metronome sound")? {
                    0 => MetronomeSound::Classic,
                    1 => MetronomeSound::Wood,
                    _ => MetronomeSound::Beep,
                },
                count_in_end: self.opt(Self::f64)?,
            },
            tracks: self.vec(min_size::TRACK, Self::track)?,
        })
    }

    fn track(&mut self) -> Result<TrackDesc, CodecError> {
        Ok(TrackDesc {
            id: self.track_id()?,
            kind: match self.tag(5, "track kind")? {
                0 => TrackKind::Audio,
                1 => TrackKind::Midi,
                2 => TrackKind::Group,
                3 => TrackKind::Return,
                _ => TrackKind::Master,
            },
            chain: self.vec(min_size::CHAIN, Self::chain_entry)?,
            output: self.opt(Self::track_id)?,
            group: self.opt(Self::track_id)?,
            sends: self.vec(min_size::SEND, |r| {
                Ok(SendDesc {
                    id: SendId(r.ulid()?),
                    to: r.track_id()?,
                    level: r.f32()?,
                    pre_fader: r.bool()?,
                })
            })?,
            volume: self.f32()?,
            pan: self.f32()?,
            mute: self.bool()?,
            solo: self.bool()?,
            audio_input: self.opt(|r| Ok((r.u16()?, r.u16()?)))?,
            monitor: self.bool()?,
            armed: self.bool()?,
            clips: self.vec(min_size::CLIP, Self::clip)?,
            automation: self.vec(min_size::AUTOMATION, Self::automation)?,
            racks: self.vec(min_size::RACK, |r| {
                Ok(RackDesc {
                    rack: r.key()?,
                    pads: r.vec(min_size::PAD, |r| {
                        Ok(PadDesc {
                            pad: DrumPadId(r.ulid()?),
                            note: r.u8()?,
                            choke_group: r.opt(Self::u8)?,
                            chain: r.vec(min_size::CHAIN, Self::chain_entry)?,
                            volume: r.f32()?,
                            pan: r.f32()?,
                            mute: r.bool()?,
                        })
                    })?,
                })
            })?,
        })
    }

    fn chain_entry(&mut self) -> Result<ChainEntry, CodecError> {
        Ok(ChainEntry {
            node: self.key()?,
            enabled: self.bool()?,
            sidechain: self.opt(Self::track_id)?,
        })
    }

    fn clip(&mut self) -> Result<ClipDesc, CodecError> {
        Ok(ClipDesc {
            id: ClipId(self.ulid()?),
            start: self.f64()?,
            length: self.f64()?,
            offset: self.f64()?,
            looping: self.opt(|r| Ok((r.f64()?, r.f64()?)))?,
            muted: self.bool()?,
            content: match self.tag(2, "clip content")? {
                0 => ClipContentDesc::Midi {
                    notes: self.vec(min_size::NOTE, |r| {
                        Ok(NoteDesc {
                            start: r.f64()?,
                            duration: r.f64()?,
                            key: r.u8()?,
                            velocity: r.f32()?,
                            release_velocity: r.f32()?,
                        })
                    })?,
                },
                _ => ClipContentDesc::Audio {
                    media: MediaId(self.ulid()?),
                    gain: self.f32()?,
                    transpose: self.f32()?,
                    fade_in: self.f64()?,
                    fade_out: self.f64()?,
                    warp: self.opt(|r| {
                        Ok(WarpDesc {
                            mode: match r.tag(2, "warp mode")? {
                                0 => WarpMode::Repitch,
                                _ => WarpMode::Complex,
                            },
                            markers: r.vec(min_size::MARKER, |r| Ok((r.f64()?, r.f64()?)))?,
                        })
                    })?,
                    fade_in_curve: self.fade()?,
                    fade_out_curve: self.fade()?,
                    reversed: self.bool()?,
                },
            },
            envelopes: self.vec(min_size::AUTOMATION, Self::automation)?,
        })
    }

    fn fade(&mut self) -> Result<FadeCurve, CodecError> {
        Ok(match self.tag(3, "fade curve")? {
            0 => FadeCurve::Linear,
            1 => FadeCurve::EqualPower,
            _ => FadeCurve::Curve {
                tension: self.f32()?,
            },
        })
    }

    fn automation(&mut self) -> Result<AutomationDesc, CodecError> {
        Ok(AutomationDesc {
            target: match self.tag(4, "automation target")? {
                0 => AutomationTarget::TrackVolume {
                    track: self.track_id()?,
                },
                1 => AutomationTarget::TrackPan {
                    track: self.track_id()?,
                },
                2 => AutomationTarget::SendLevel {
                    send: SendId(self.ulid()?),
                },
                _ => AutomationTarget::DeviceParam {
                    device: DeviceId(self.ulid()?),
                    param: ParamId(self.u32()?),
                },
            },
            resolved: match self.tag(4, "resolved target")? {
                0 => ResolvedTarget::TrackVolume,
                1 => ResolvedTarget::TrackPan,
                2 => ResolvedTarget::Send {
                    send: SendId(self.ulid()?),
                },
                _ => ResolvedTarget::Node {
                    node: self.key()?,
                    param: ParamId(self.u32()?),
                },
            },
            points: self.vec(min_size::POINT, |r| {
                Ok((
                    r.f64()?,
                    r.f64()?,
                    match r.tag(3, "curve shape")? {
                        0 => CurveShape::Linear,
                        1 => CurveShape::Step,
                        _ => CurveShape::Curve { tension: r.f32()? },
                    },
                ))
            })?,
            mapping: ParamMapping {
                min: self.f64()?,
                max: self.f64()?,
                scale: match self.tag(4, "param scale")? {
                    0 => ParamScale::Linear,
                    1 => ParamScale::Log,
                    2 => ParamScale::Power {
                        exponent: self.f64()?,
                    },
                    _ => ParamScale::Fader,
                },
                steps: self.opt(Self::u32)?,
            },
        })
    }
}
