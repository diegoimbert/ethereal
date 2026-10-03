//! Shared-memory block exchange between the host node and the helper's audio thread.
//!
//! Layout (version 3), each array starting on a 64-byte boundary at a fixed offset derived
//! from the capacities stored in the header:
//!
//! | section        | size                                                   |
//! |----------------|--------------------------------------------------------|
//! | [`Header`]     | capacities (incl. `sc_channels`) + sync atomics        |
//! | [`Block`]      | the current block's plain request/result fields (never aliased with the atomics; `sidechain` = 1 when the block carries a sidechain signal) |
//! | input audio    | `in_channels × max_frames` planar `f32`                |
//! | sidechain audio| `sc_channels × max_frames` planar `f32` (CONTRACTS §12.14; 0 bytes without a sidechain input) |
//! | output audio   | `out_channels × max_frames` planar `f32`               |
//! | input events   | `max_in_events` [`WireEvent`]s                         |
//! | output events  | `max_out_events` [`WireEvent`]s                        |
//!
//! Version history: 2 = before sidechains; 3 = `plugin-sidechain` (the `sc_channels` header
//! field in the former padding, the `Block::sidechain` flag, the sidechain audio array).
//!
//! Protocol (one block in flight at most):
//! 1. host fills the request fields + input audio/events, stores `posted = seq` (Release),
//!    posts the semaphore;
//! 2. helper wakes, reads `posted` (Acquire), processes, writes the result fields + output
//!    audio/events, stores `done = seq` (Release);
//! 3. host sees `done >= seq` (Acquire) at its next block and reads the result.
//!
//! Each side only touches the non-atomic fields during its own phase, so the atomics order
//! everything. Events and transport use explicit `repr(C)` encodings (never Rust layouts).

use std::sync::atomic::{AtomicU64, Ordering};

use ether_core::event::{EventKind, ProcessEvent};
use ether_core::protocol::model::{NoteExpressionKind, ParamId, TimeSignature};
use ether_core::transport::TransportInfo;
use shared_memory::{Shmem, ShmemConf, ShmemError};

const MAGIC: u32 = 0x4554_5342; // "ETSB"
/// Layout version, checked by the helper on open. Bump on any change to the region layout.
const VERSION: u32 = 3;
const ALIGN: usize = 64;

/// Block transport, `repr(C)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WireTransport {
    flags: u32,
    ts_num: u32,
    ts_den: u32,
    _pad: u32,
    sample_time: u64,
    position: f64,
    seconds: f64,
    bpm: f64,
    beats_per_sample: f64,
    bar_start: f64,
    loop_start: f64,
    loop_end: f64,
}

const T_PLAYING: u32 = 1;
const T_RECORDING: u32 = 2;
const T_LOOP: u32 = 4;

impl WireTransport {
    pub fn encode(t: &TransportInfo) -> Self {
        let mut flags = 0;
        if t.playing {
            flags |= T_PLAYING;
        }
        if t.recording {
            flags |= T_RECORDING;
        }
        if t.loop_active {
            flags |= T_LOOP;
        }
        Self {
            flags,
            ts_num: u32::from(t.time_signature.numerator),
            ts_den: u32::from(t.time_signature.denominator),
            _pad: 0,
            sample_time: t.sample_time,
            position: t.position,
            seconds: t.seconds,
            bpm: t.bpm,
            beats_per_sample: t.beats_per_sample,
            bar_start: t.bar_start,
            loop_start: t.loop_start,
            loop_end: t.loop_end,
        }
    }

    pub fn decode(&self) -> TransportInfo {
        TransportInfo {
            playing: self.flags & T_PLAYING != 0,
            recording: self.flags & T_RECORDING != 0,
            sample_time: self.sample_time,
            position: self.position,
            seconds: self.seconds,
            bpm: self.bpm,
            beats_per_sample: self.beats_per_sample,
            time_signature: TimeSignature {
                numerator: self.ts_num.min(255) as u8,
                denominator: self.ts_den.min(255) as u8,
            },
            bar_start: self.bar_start,
            loop_active: self.flags & T_LOOP != 0,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
        }
    }
}

/// One event, `repr(C)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct WireEvent {
    offset: u32,
    kind: u32,
    /// note id / param id / packed MIDI bytes.
    a: u32,
    /// channel | key << 8.
    b: u32,
    /// velocity / param value.
    value: f64,
}

const E_NOTE_ON: u32 = 1;
const E_NOTE_OFF: u32 = 2;
const E_CHOKE: u32 = 3;
const E_ALL_OFF: u32 = 4;
const E_PARAM: u32 = 5;
const E_MIDI: u32 = 6;
/// v0.3 (`midi-expression`/`mpe`): `a` = note id, `b` = channel | key << 8 | kind << 16
/// (`NoteExpressionKind`: 0 pitch, 1 pressure, 2 timbre), `value` = the value. A new kind
/// code, not a layout change (both ends come from one build).
const E_NOTE_EXPRESSION: u32 = 7;

impl WireEvent {
    pub fn encode(e: &ProcessEvent) -> Self {
        let ev = |kind, a, b, value| Self {
            offset: e.offset,
            kind,
            a,
            b,
            value,
        };
        let ck = |channel: u8, key: u8| u32::from(channel) | (u32::from(key) << 8);
        match e.kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => ev(E_NOTE_ON, note_id, ck(channel, key), f64::from(velocity)),
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => ev(E_NOTE_OFF, note_id, ck(channel, key), f64::from(velocity)),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => ev(E_CHOKE, note_id, ck(channel, key), 0.0),
            EventKind::AllNotesOff => ev(E_ALL_OFF, 0, 0, 0.0),
            EventKind::Param { param, value } => ev(E_PARAM, param.0, 0, value),
            EventKind::Midi { data } => ev(
                E_MIDI,
                u32::from_le_bytes([data[0], data[1], data[2], 0]),
                0,
                0.0,
            ),
            EventKind::NoteExpression {
                note_id,
                channel,
                key,
                expression,
                value,
            } => {
                let kind: u32 = match expression {
                    NoteExpressionKind::Pitch => 0,
                    NoteExpressionKind::Pressure => 1,
                    NoteExpressionKind::Timbre => 2,
                };
                ev(
                    E_NOTE_EXPRESSION,
                    note_id,
                    ck(channel, key) | (kind << 16),
                    f64::from(value),
                )
            }
        }
    }

    pub fn decode(&self) -> Option<ProcessEvent> {
        let channel = (self.b & 0xff) as u8;
        let key = ((self.b >> 8) & 0xff) as u8;
        let kind = match self.kind {
            E_NOTE_ON => EventKind::NoteOn {
                note_id: self.a,
                channel,
                key,
                velocity: self.value as f32,
            },
            E_NOTE_OFF => EventKind::NoteOff {
                note_id: self.a,
                channel,
                key,
                velocity: self.value as f32,
            },
            E_CHOKE => EventKind::NoteChoke {
                note_id: self.a,
                channel,
                key,
            },
            E_ALL_OFF => EventKind::AllNotesOff,
            E_PARAM => EventKind::Param {
                param: ParamId(self.a),
                value: self.value,
            },
            E_MIDI => {
                let [d0, d1, d2, _] = self.a.to_le_bytes();
                EventKind::Midi { data: [d0, d1, d2] }
            }
            E_NOTE_EXPRESSION => EventKind::NoteExpression {
                note_id: self.a,
                channel,
                key,
                expression: match (self.b >> 16) & 0xff {
                    0 => NoteExpressionKind::Pitch,
                    1 => NoteExpressionKind::Pressure,
                    2 => NoteExpressionKind::Timbre,
                    _ => return None,
                },
                value: self.value as f32,
            },
            _ => return None,
        };
        Some(ProcessEvent {
            offset: self.offset,
            kind,
        })
    }
}

/// Start of the region.
#[repr(C)]
pub(crate) struct Header {
    magic: u32,
    version: u32,
    pub max_frames: u32,
    pub in_channels: u32,
    pub out_channels: u32,
    pub max_in_events: u32,
    pub max_out_events: u32,
    /// Channels of the sidechain audio array (the node's `sidechain_inputs`; 0 = none).
    pub sc_channels: u32,
    /// Last block sequence number posted by the host (0 = none yet).
    pub posted: AtomicU64,
    /// Last block sequence number completed by the helper.
    pub done: AtomicU64,
}

/// The current block's plain (non-atomic) fields, in their own struct so that exclusive
/// access to them never aliases the header's atomics.
#[repr(C)]
pub(crate) struct Block {
    // --- request (host phase) ---
    pub frames: u32,
    pub n_in_events: u32,
    /// 1 = call `Node::reset` before processing.
    pub reset: u32,
    /// 1 = the sidechain audio array holds this block's sidechain signal: the helper calls
    /// `Node::process_sidechain`; 0 = no source (`Node::process`, the aux bus gets silence).
    pub sidechain: u32,
    pub transport: WireTransport,
    // --- result (helper phase) ---
    pub n_out_events: u32,
    /// 0 = Continue, 1 = Silent.
    pub status: u32,
}

/// Capacities and byte offsets of the arrays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    pub max_frames: usize,
    pub in_channels: usize,
    pub out_channels: usize,
    pub max_in_events: usize,
    pub max_out_events: usize,
    pub sc_channels: usize,
    block: usize,
    in_audio: usize,
    sc_audio: usize,
    out_audio: usize,
    in_events: usize,
    out_events: usize,
    pub size: usize,
}

fn align(n: usize) -> usize {
    n.div_ceil(ALIGN) * ALIGN
}

impl Layout {
    /// A layout without sidechain channels.
    #[cfg(test)]
    pub fn new(
        max_frames: usize,
        in_channels: usize,
        out_channels: usize,
        max_in_events: usize,
        max_out_events: usize,
    ) -> Self {
        Self::with_sidechain(
            max_frames,
            in_channels,
            out_channels,
            max_in_events,
            max_out_events,
            0,
        )
    }

    /// [`Layout::new`] plus `sc_channels` sidechain channels (after the main inputs).
    pub fn with_sidechain(
        max_frames: usize,
        in_channels: usize,
        out_channels: usize,
        max_in_events: usize,
        max_out_events: usize,
        sc_channels: usize,
    ) -> Self {
        let f = std::mem::size_of::<f32>();
        let e = std::mem::size_of::<WireEvent>();
        let block = align(std::mem::size_of::<Header>());
        let in_audio = align(block + std::mem::size_of::<Block>());
        let sc_audio = align(in_audio + in_channels * max_frames * f);
        let out_audio = align(sc_audio + sc_channels * max_frames * f);
        let in_events = align(out_audio + out_channels * max_frames * f);
        let out_events = align(in_events + max_in_events * e);
        let size = align(out_events + max_out_events * e);
        Self {
            max_frames,
            in_channels,
            out_channels,
            max_in_events,
            max_out_events,
            sc_channels,
            block,
            in_audio,
            sc_audio,
            out_audio,
            in_events,
            out_events,
            size,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RegionError {
    #[error("shared memory: {0}")]
    Shmem(#[from] ShmemError),
    #[error("shared memory region is invalid")]
    Invalid,
}

/// A mapped sandbox region. The creator owns the name (unlinked on drop or by `unlink`).
pub(crate) struct Region {
    shm: Shmem,
    layout: Layout,
}

impl std::fmt::Debug for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Region")
            .field("name", &self.shm.get_os_id())
            .field("layout", &self.layout)
            .finish()
    }
}

// SAFETY: the mapping is plain memory; cross-thread/process access is ordered by the header
// atomics (see module docs) and each process has a single user per side.
unsafe impl Send for Region {}

impl Region {
    /// Create and initialize. A stale object with the same name (left by a crash) is removed
    /// first.
    pub fn create(name: &str, layout: Layout) -> Result<Self, RegionError> {
        if let Ok(mut stale) = ShmemConf::new().os_id(name).open() {
            stale.set_owner(true); // unlinked on drop
        }
        let shm = ShmemConf::new().os_id(name).size(layout.size).create()?;
        let region = Self { shm, layout };
        let h = region.header_ptr();
        // SAFETY: the mapping is at least `layout.size` bytes, page-aligned, zero-filled
        // (fresh ftruncate) and not yet shared with anyone.
        unsafe {
            (*h).magic = MAGIC;
            (*h).version = VERSION;
            (*h).max_frames = layout.max_frames as u32;
            (*h).in_channels = layout.in_channels as u32;
            (*h).out_channels = layout.out_channels as u32;
            (*h).max_in_events = layout.max_in_events as u32;
            (*h).max_out_events = layout.max_out_events as u32;
            (*h).sc_channels = layout.sc_channels as u32;
        }
        Ok(region)
    }

    /// Map an existing region and validate its header.
    pub fn open(name: &str) -> Result<Self, RegionError> {
        let shm = ShmemConf::new().os_id(name).open()?;
        if shm.len() < std::mem::size_of::<Header>() {
            return Err(RegionError::Invalid);
        }
        let h = shm.as_ptr() as *const Header;
        // SAFETY: at least a header's worth of mapped bytes (checked above); the creator
        // initialized it before handing out the name.
        let layout = unsafe {
            if (*h).magic != MAGIC || (*h).version != VERSION {
                return Err(RegionError::Invalid);
            }
            Layout::with_sidechain(
                (*h).max_frames as usize,
                (*h).in_channels as usize,
                (*h).out_channels as usize,
                (*h).max_in_events as usize,
                (*h).max_out_events as usize,
                (*h).sc_channels as usize,
            )
        };
        if shm.len() < layout.size {
            return Err(RegionError::Invalid);
        }
        Ok(Self { shm, layout })
    }

    /// Remove the name once the peer has mapped it; the memory lives on while mapped.
    pub fn unlink(&mut self) {
        if self.shm.is_owner() {
            // Re-open by name purely to unlink it, then disown ours.
            if let Ok(mut other) = ShmemConf::new().os_id(self.shm.get_os_id()).open() {
                other.set_owner(true);
            }
            self.shm.set_owner(false);
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    fn header_ptr(&self) -> *mut Header {
        self.shm.as_ptr() as *mut Header
    }

    /// Shared view of the header (atomics).
    pub fn header(&self) -> &Header {
        // SAFETY: valid, initialized header for the life of the mapping.
        unsafe { &*self.header_ptr() }
    }

    /// Exclusive view of the current block's plain fields. Only call during this side's
    /// phase (see module docs).
    #[allow(clippy::mut_from_ref)]
    pub fn block(&self) -> &mut Block {
        // SAFETY: `layout.block` is inside the mapping, aligned (ALIGN) and holds plain data;
        // the protocol gives this side exclusive access during its phase. It does not overlap
        // the header's atomics.
        unsafe { &mut *(self.shm.as_ptr().add(self.layout.block) as *mut Block) }
    }

    #[allow(clippy::mut_from_ref)]
    fn slice<T>(&self, offset: usize, len: usize) -> &mut [T] {
        // SAFETY: offsets/lengths come from `layout`, which fits the mapping; `T` is `f32`
        // or `WireEvent` (plain data, aligned by `ALIGN`); exclusivity per protocol phase.
        unsafe { std::slice::from_raw_parts_mut(self.shm.as_ptr().add(offset) as *mut T, len) }
    }

    /// Planar input audio, `in_channels * max_frames`. Host phase: write; helper: read.
    #[allow(clippy::mut_from_ref)]
    pub fn in_audio(&self) -> &mut [f32] {
        let l = &self.layout;
        self.slice(l.in_audio, l.in_channels * l.max_frames)
    }

    /// Planar sidechain audio, `sc_channels * max_frames`. Host phase: write; helper: read.
    #[allow(clippy::mut_from_ref)]
    pub fn sc_audio(&self) -> &mut [f32] {
        let l = &self.layout;
        self.slice(l.sc_audio, l.sc_channels * l.max_frames)
    }

    #[allow(clippy::mut_from_ref)]
    pub fn out_audio(&self) -> &mut [f32] {
        let l = &self.layout;
        self.slice(l.out_audio, l.out_channels * l.max_frames)
    }

    #[allow(clippy::mut_from_ref)]
    pub fn in_events(&self) -> &mut [WireEvent] {
        let l = &self.layout;
        self.slice(l.in_events, l.max_in_events)
    }

    #[allow(clippy::mut_from_ref)]
    pub fn out_events(&self) -> &mut [WireEvent] {
        let l = &self.layout;
        self.slice(l.out_events, l.max_out_events)
    }

    pub fn load_done(&self) -> u64 {
        self.header().done.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::os_name;

    #[test]
    fn events_roundtrip() {
        let kinds = [
            EventKind::NoteOn {
                note_id: 9,
                channel: 3,
                key: 64,
                velocity: 0.5,
            },
            EventKind::NoteOff {
                note_id: 9,
                channel: 3,
                key: 64,
                velocity: 0.25,
            },
            EventKind::NoteChoke {
                note_id: 1,
                channel: 15,
                key: 127,
            },
            EventKind::AllNotesOff,
            EventKind::Param {
                param: ParamId(42),
                value: -1.5,
            },
            EventKind::Midi {
                data: [0xB0, 7, 127],
            },
            EventKind::NoteExpression {
                note_id: 9,
                channel: 2,
                key: 61,
                expression: NoteExpressionKind::Timbre,
                value: 0.75,
            },
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            let e = ProcessEvent {
                offset: i as u32,
                kind,
            };
            assert_eq!(WireEvent::encode(&e).decode(), Some(e));
        }
        let mut t = TransportInfo::STOPPED;
        t.playing = true;
        t.loop_active = true;
        t.bpm = 133.0;
        t.sample_time = 1 << 40;
        assert_eq!(WireTransport::encode(&t).decode(), t);
    }

    #[test]
    fn region_create_open_unlink() {
        let name = os_name("sandbox-unit", std::process::id(), "shm-test");
        let layout = Layout::new(128, 2, 2, 16, 32);
        let mut a = Region::create(&name, layout).unwrap();
        // Creating again over a live name replaces it (stale cleanup).
        let mut a2 = Region::create(&name, layout).unwrap();
        a2.in_audio()[5] = 0.5;
        let b = Region::open(&name).unwrap();
        assert_eq!(*b.layout(), layout);
        assert_eq!(b.in_audio()[5], 0.5);
        a2.unlink();
        a.unlink();
        assert!(Region::open(&name).is_err());
        // Still shared after unlink.
        b.out_audio()[3] = 2.0;
        assert_eq!(a2.out_audio()[3], 2.0);
        assert!(b.sc_audio().is_empty());
    }

    #[test]
    fn sidechain_array_sits_between_main_inputs_and_outputs() {
        let plain = Layout::new(100, 2, 2, 8, 8);
        let sc = Layout::with_sidechain(100, 2, 2, 8, 8, 2);
        assert_eq!(plain.sc_channels, 0);
        assert_eq!(plain.sc_audio, plain.out_audio);
        assert!(sc.sc_audio >= sc.in_audio + 2 * 100 * 4);
        assert!(sc.out_audio >= sc.sc_audio + 2 * 100 * 4);
        assert_eq!(sc.sc_audio % ALIGN, 0);
        assert!(sc.size > plain.size);

        let name = os_name("sandbox-unit", std::process::id(), "shm-sc-test");
        let mut a = Region::create(&name, sc).unwrap();
        let b = Region::open(&name).unwrap();
        a.unlink();
        assert_eq!(*b.layout(), sc);
        a.in_audio().fill(1.0);
        a.out_audio().fill(3.0);
        a.sc_audio()[150] = 0.5;
        assert_eq!(b.sc_audio().len(), 200);
        assert_eq!(b.sc_audio()[150], 0.5);
        // The arrays don't overlap.
        assert!(b.in_audio().iter().all(|&x| x == 1.0));
        assert!(b.out_audio().iter().all(|&x| x == 3.0));
    }

    #[test]
    fn a_region_of_another_version_is_refused() {
        let name = os_name("sandbox-unit", std::process::id(), "shm-ver-test");
        let mut a = Region::create(&name, Layout::new(16, 1, 1, 4, 4)).unwrap();
        // SAFETY: test-only; nobody else maps the region yet.
        unsafe { (*a.header_ptr()).version = VERSION - 1 };
        assert!(matches!(Region::open(&name), Err(RegionError::Invalid)));
        a.unlink();
    }
}
