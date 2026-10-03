//! Hardware I/O inside the graph (v0.3, contracts-4; owned by the `external-instrument`
//! node; CONTRACTS.md §13.7): External Instrument (MIDI out + audio return) and External
//! Audio Effect (audio send + return).
//!
//! The controller compiles every external device of a track into a [`HwIoDesc`]
//! (`TrackDesc::hw_io`, keyed by the device's chain node). [`EngineHandle::publish`] builds
//! a [`HwIoRt`] for the snapshot (buffers allocated there, off the audio thread) and swaps it
//! in with the snapshot. Per sub-block, the engine (shared touch in `engine.rs`):
//! 1. before the track jobs, copies each desc's `audio_return` hardware input channels
//!    (from `Engine::process`'s `inputs`, the same buffers recording/monitoring read) into
//!    the node's return buffer ([`HwIoRt::gather_returns_at`]); a mono return feeds both
//!    sides, channels the host doesn't have read as silence (a missing port is silent and
//!    resumes when it reappears);
//! 2. runs the external device node with [`crate::Node::process_sidechain`]: the
//!    sidechain is the return (2 channels), and an effect gets two extra output channels,
//!    `outputs[2..4]`, which are its hardware send ([`HwEntry`]): the node writes its input
//!    scaled by its send gain there. The node outputs the return (instrument) or mixes it
//!    with its (latency-delayed) dry input by its `MIX` param (effect);
//! 3. the instrument's input events (notes, raw MIDI, all-notes-off) become MIDI bytes on
//!    `routing.midi_channel`, stamped with their engine sample time ([`HwEntry::push_events`]);
//! 4. after the jobs and the master bus, adds each send buffer to its `audio_send` hardware
//!    output channels and flushes the MIDI to a lock-free ring of [`HwMidiEvent`]s
//!    ([`HwIo::write_sends`]), drained by the host's MIDI output thread
//!    ([`HwIoIo::midi`], taken once with [`EngineHandle::take_hw_io`]), which maps `node` to
//!    the device's `midi_out` port and sends each message when its sample reaches the
//!    speakers. The web has no hardware MIDI out (devices stay silent there).
//!
//! # Latency
//! The device node reports its `LATENCY` param (ms → samples) as `Node::latency`, so PDC
//! delays the rest of the mix by the hardware round trip and the returned audio lines up.
//! The effect delays its dry signal by the same amount, so dry and wet are aligned too.
//!
//! **Measuring** ([`EngineHandle::measure_hw_latency`], `External::MeasureLatency`): on the
//! next sub-block that has the node, an effect's send carries a single-sample click
//! ([`MEASURE_CLICK`]; the program is muted on the send while measuring) and an instrument
//! sends a note-on (key 60, velocity 127). The return is then scanned for the first sample
//! above [`MEASURE_THRESHOLD`]; for an effect the peak within [`MEASURE_PEAK_WINDOW`] samples
//! after it is taken (converter filters ring before their peak), for an instrument the
//! first crossing (synth attacks only get later). The round trip in samples comes back
//! through [`EngineHandle::poll_hw_latency`]; nothing above the threshold within
//! [`MEASURE_TIMEOUT_MS`] (or the node never showing up) reports `None`.
//!
//! RT rules: buffers are allocated at publish; the audio thread only copies, adds, scans and
//! pushes to pre-allocated rings.

use ether_protocol::model::ExternalRouting;
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::engine::EngineHandle;
use crate::event::{EventKind, ProcessEvent};
use crate::graph::RenderGraphDesc;
use crate::node::NodeKey;

/// MIDI messages queued towards the host's MIDI output thread.
pub const HW_MIDI_CAPACITY: usize = 8192;
/// MIDI messages one external instrument stages per sub-block (more are dropped and counted
/// as an overflow).
pub const MAX_HW_MIDI_PER_BLOCK: usize = 512;
/// Pending latency measurement requests / results.
const MEASURE_QUEUE: usize = 16;
/// Amplitude of the measurement click on the send.
pub const MEASURE_CLICK: f32 = 0.5;
/// Return level (absolute) that counts as the click (or note) coming back.
pub const MEASURE_THRESHOLD: f32 = 0.03;
/// Samples after the first crossing searched for the click's peak (effects).
pub const MEASURE_PEAK_WINDOW: u64 = 64;
/// Nothing back within this long = `MeasureFailed` (CONTRACTS.md §13.7: 2 s).
pub const MEASURE_TIMEOUT_MS: u64 = 2000;
/// Key / velocity of the measurement note (instruments).
pub const MEASURE_KEY: u8 = 60;
const MEASURE_VELOCITY: u8 = 127;
/// Length of the measurement note when nothing comes back earlier (samples are derived at
/// the engine rate).
const MEASURE_NOTE_MS: u64 = 250;

/// One external device of a track, compiled for the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HwIoDesc {
    /// The device's chain node.
    pub node: NodeKey,
    /// Hardware routing (ports and channels) as stored in the document.
    pub routing: ExternalRouting,
}

/// A MIDI message an External Instrument sends to hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwMidiEvent {
    pub node: NodeKey,
    /// Engine sample time of the message.
    pub frame: u64,
    pub data: [u8; 3],
}

/// Result of a latency measurement ([`EngineHandle::poll_hw_latency`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwLatencyResult {
    pub node: NodeKey,
    /// Round trip in samples; `None` = nothing came back within [`MEASURE_TIMEOUT_MS`].
    pub samples: Option<u32>,
}

/// Per-node hardware state of a snapshot: the return and send buffers of one sub-block and
/// the instrument's staged MIDI.
#[derive(Debug)]
pub struct HwEntry {
    node: NodeKey,
    routing: ExternalRouting,
    ret: [Vec<f32>; 2],
    send: [Vec<f32>; 2],
    /// The node wrote `send` this sub-block (bypassed or missing nodes don't).
    sent: bool,
    midi: Vec<HwMidiEvent>,
    overflow: bool,
}

impl HwEntry {
    fn new(desc: &HwIoDesc, max_block: usize) -> Self {
        let has_send = desc.routing.audio_send.is_some();
        let send_len = if has_send { max_block } else { 0 };
        let midi = if desc.routing.midi_out.is_some() {
            MAX_HW_MIDI_PER_BLOCK
        } else {
            0
        };
        Self {
            node: desc.node,
            routing: desc.routing.clone(),
            ret: [vec![0.0; max_block], vec![0.0; max_block]],
            send: [vec![0.0; send_len], vec![0.0; send_len]],
            sent: false,
            midi: Vec::with_capacity(midi),
            overflow: false,
        }
    }

    pub fn node(&self) -> NodeKey {
        self.node
    }

    pub fn routing(&self) -> &ExternalRouting {
        &self.routing
    }

    /// Whether the device sends audio to hardware (an effect with an `audio_send`).
    pub fn has_send(&self) -> bool {
        self.routing.audio_send.is_some()
    }

    /// RT. The return of this sub-block (`n` frames, stereo).
    pub fn returns(&self, n: usize) -> [&[f32]; 2] {
        let n = n.min(self.ret[0].len());
        [&self.ret[0][..n], &self.ret[1][..n]]
    }

    /// RT. The send buffers of this sub-block, for the node to write (marks the send as
    /// written). Empty slices when the device has no send.
    pub fn send_mut(&mut self, n: usize) -> [&mut [f32]; 2] {
        let n = n.min(self.send[0].len());
        self.sent = n > 0;
        let [l, r] = &mut self.send;
        [&mut l[..n], &mut r[..n]]
    }

    /// RT. The return (as the sidechain) and the send (as outputs) at once, for the
    /// engine's `process_sidechain` call.
    pub fn split(&mut self, n: usize) -> ([&[f32]; 2], [&mut [f32]; 2]) {
        let rn = n.min(self.ret[0].len());
        let sn = n.min(self.send[0].len());
        self.sent = sn > 0;
        let [rl, rr] = &self.ret;
        let [sl, sr] = &mut self.send;
        ([&rl[..rn], &rr[..rn]], [&mut sl[..sn], &mut sr[..sn]])
    }

    /// RT. Stage an instrument's input events as MIDI on its channel, at engine sample
    /// `frame0 + offset`. Params and note expressions are not sent.
    pub fn push_events(&mut self, events: &[ProcessEvent], frame0: u64) {
        if self.routing.midi_out.is_none() {
            return;
        }
        let ch = self.routing.midi_channel.clamp(1, 16) - 1;
        let vel = |v: f32| (v.clamp(0.0, 1.0) * 127.0).round() as u8;
        for e in events {
            let data = match e.kind {
                EventKind::NoteOn { key, velocity, .. } => {
                    [0x90 | ch, key & 0x7f, vel(velocity).max(1)]
                }
                EventKind::NoteOff { key, velocity, .. } => [0x80 | ch, key & 0x7f, vel(velocity)],
                EventKind::NoteChoke { key, .. } => [0x80 | ch, key & 0x7f, 0],
                EventKind::AllNotesOff => [0xb0 | ch, 123, 0],
                EventKind::Midi { data } if (0x80..0xf0).contains(&data[0]) => {
                    [(data[0] & 0xf0) | ch, data[1] & 0x7f, data[2] & 0x7f]
                }
                _ => continue,
            };
            self.stage(frame0 + u64::from(e.offset), data);
        }
    }

    fn stage(&mut self, frame: u64, data: [u8; 3]) {
        if self.midi.len() < self.midi.capacity() {
            self.midi.push(HwMidiEvent {
                node: self.node,
                frame,
                data,
            });
        } else {
            self.overflow = true;
        }
    }
}

/// Engine-side state of one snapshot (return/send buffers per external device), built by
/// [`HwIoRt::prepare`] off the audio thread.
#[derive(Debug, Default)]
pub struct HwIoRt {
    /// Sorted by node.
    entries: Vec<HwEntry>,
}

impl HwIoRt {
    /// Non-RT: allocate buffers for a new snapshot's descs.
    pub fn prepare(&mut self, descs: &[HwIoDesc], max_block: usize) {
        self.entries = descs.iter().map(|d| HwEntry::new(d, max_block)).collect();
        self.entries.sort_by_key(|e| e.node);
        self.entries.dedup_by_key(|e| e.node);
    }

    /// Non-RT: the state for every track of `desc`, or `None` without external devices.
    pub fn for_graph(desc: &RenderGraphDesc, max_block: usize) -> Option<Box<Self>> {
        let descs: Vec<HwIoDesc> = desc
            .tracks
            .iter()
            .flat_map(|t| t.hw_io.iter().cloned())
            .collect();
        if descs.is_empty() {
            return None;
        }
        let mut rt = Self::default();
        rt.prepare(&descs, max_block);
        Some(Box::new(rt))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[HwEntry] {
        &self.entries
    }

    fn index(&self, node: NodeKey) -> Option<usize> {
        self.entries.binary_search_by_key(&node, |e| e.node).ok()
    }

    /// RT. The entry of `node`.
    pub fn entry_mut(&mut self, node: NodeKey) -> Option<&mut HwEntry> {
        let i = self.index(node)?;
        Some(&mut self.entries[i])
    }

    /// RT: copy hardware inputs into the return buffers (`frames` from the start of every
    /// input).
    pub fn gather_returns(&mut self, inputs: &[&[f32]], frames: usize) {
        self.gather_returns_at(inputs, 0, frames);
    }

    /// RT: copy hardware inputs `[off, off + frames)` into the return buffers. Channels the
    /// host doesn't provide are silent; a mono return feeds both sides. Also starts the
    /// sub-block (sends not yet written).
    pub fn gather_returns_at(&mut self, inputs: &[&[f32]], off: usize, frames: usize) {
        for e in self.entries.iter_mut() {
            e.sent = false;
            let n = frames.min(e.ret[0].len());
            let read = |ch: u16, out: &mut [f32]| match inputs.get(usize::from(ch)) {
                Some(input) if input.len() >= off + n => out.copy_from_slice(&input[off..off + n]),
                Some(input) => {
                    let avail = input.len().saturating_sub(off).min(n);
                    out[..avail].copy_from_slice(&input[off.min(input.len())..][..avail]);
                    out[avail..].fill(0.0);
                }
                None => out.fill(0.0),
            };
            let [l, r] = &mut e.ret;
            match e.routing.audio_return {
                Some(c) if c.count >= 2 => {
                    read(c.first, &mut l[..n]);
                    match c.first.checked_add(1) {
                        Some(second) => read(second, &mut r[..n]),
                        None => r[..n].fill(0.0),
                    }
                }
                Some(c) => {
                    read(c.first, &mut l[..n]);
                    r[..n].copy_from_slice(&l[..n]);
                }
                None => {
                    l[..n].fill(0.0);
                    r[..n].fill(0.0);
                }
            }
        }
    }

    /// RT: an effect's input for the hardware send, copied as is (the engine lets the node
    /// write its send instead, see [`HwEntry::split`]; this is for hosts driving nodes by
    /// hand). A mono input feeds both sides.
    pub fn capture_send(&mut self, node: NodeKey, input: &[&[f32]], frames: usize) {
        let Some(e) = self.entry_mut(node) else {
            return;
        };
        let [l, r] = e.send_mut(frames);
        let n = l.len();
        match input {
            [a, b, ..] => {
                l.copy_from_slice(&a[..n]);
                r.copy_from_slice(&b[..n]);
            }
            [a] => {
                l.copy_from_slice(&a[..n]);
                r.copy_from_slice(&a[..n]);
            }
            [] => {
                l.fill(0.0);
                r.fill(0.0);
            }
        }
    }

    /// RT: add the send buffers to the hardware outputs (`frames` from the start).
    pub fn write_sends(&mut self, outputs: &mut [&mut [f32]], frames: usize) {
        self.write_sends_at(outputs, 0, frames);
    }

    /// RT: add the send buffers written this sub-block to hardware outputs
    /// `[off, off + frames)`. Missing output channels are skipped; a mono send gets the
    /// average of both sides.
    pub fn write_sends_at(&mut self, outputs: &mut [&mut [f32]], off: usize, frames: usize) {
        for e in self.entries.iter_mut() {
            let Some(c) = e.routing.audio_send else {
                continue;
            };
            if !e.sent {
                continue;
            }
            let n = frames.min(e.send[0].len());
            let mut add = |ch: usize, f: &dyn Fn(usize) -> f32| {
                if let Some(out) = outputs.get_mut(ch) {
                    let end = (off + n).min(out.len());
                    for (i, d) in out[off.min(end)..end].iter_mut().enumerate() {
                        *d += f(i);
                    }
                }
            };
            let [l, r] = &e.send;
            let first = usize::from(c.first);
            if c.count >= 2 {
                add(first, &|i| l[i]);
                add(first + 1, &|i| r[i]);
            } else {
                add(first, &|i| (l[i] + r[i]) * 0.5);
            }
        }
    }

    /// RT: hand the staged MIDI of every entry to `f` (oldest first per entry) and clear it.
    /// Returns whether an entry dropped messages since the last call.
    pub fn drain_midi(&mut self, mut f: impl FnMut(HwMidiEvent)) -> bool {
        let mut overflow = false;
        for e in self.entries.iter_mut() {
            for m in e.midi.drain(..) {
                f(m);
            }
            overflow |= std::mem::take(&mut e.overflow);
        }
        overflow
    }

    /// A view for concurrent track jobs (see [`HwIoTable`]).
    pub(crate) fn table(&mut self) -> HwIoTable {
        HwIoTable {
            ptr: self.entries.as_mut_ptr(),
            len: self.entries.len(),
        }
    }
}

/// RT. Keyed access to a [`HwIoRt`]'s entries from concurrent track jobs, like the engine's
/// node table: every node is in one chain, so each entry is reached by one job only.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HwIoTable {
    ptr: *mut HwEntry,
    len: usize,
}

impl Default for HwIoTable {
    fn default() -> Self {
        Self {
            ptr: std::ptr::NonNull::dangling().as_ptr(),
            len: 0,
        }
    }
}

impl HwIoTable {
    /// The entry of `node`.
    ///
    /// # Safety
    /// The table's `HwIoRt` outlives the use and isn't touched through another path while
    /// jobs run; `node` belongs to the caller's track (each node is in one chain, so no two
    /// jobs get the same entry), and the caller holds one returned reference at a time.
    pub(crate) unsafe fn get<'a>(self, node: NodeKey) -> Option<&'a mut HwEntry> {
        if self.len == 0 {
            return None;
        }
        // Binary search reading only the immutable `node` fields through raw pointers.
        let (mut lo, mut hi) = (0usize, self.len);
        while lo < hi {
            let mid = (lo + hi) / 2;
            // SAFETY: `mid < len`; `node` is never written while jobs run.
            let k = unsafe { std::ptr::addr_of!((*self.ptr.add(mid)).node).read() };
            match k.cmp(&node) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                // SAFETY: by the contract above, this entry is this job's alone.
                std::cmp::Ordering::Equal => return Some(unsafe { &mut *self.ptr.add(mid) }),
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Waiting for a sub-block that has the node (`waited` samples so far).
    Pending { waited: u64 },
    /// Click / note sent at `t0`.
    Sent { t0: u64 },
    /// Crossed the threshold; looking for the peak until `until`.
    Peak {
        t0: u64,
        at: u64,
        level: f32,
        until: u64,
    },
}

#[derive(Clone, Copy, Debug)]
struct Measure {
    node: NodeKey,
    phase: Phase,
    /// The note-off of an instrument's measurement note is still due.
    note_on: bool,
    instrument: bool,
}

/// Audio-thread half: the current snapshot's [`HwIoRt`], the MIDI ring and the latency
/// measurement. Owned by `Engine`.
pub struct HwIo {
    rt: Option<Box<HwIoRt>>,
    midi: Producer<HwMidiEvent>,
    requests: Consumer<NodeKey>,
    results: Producer<HwLatencyResult>,
    measure: Option<Measure>,
    /// An instrument's measurement note to release.
    note_off: Option<NodeKey>,
    timeout: u64,
    note_len: u64,
    /// MIDI messages dropped (ring full, or more than [`MAX_HW_MIDI_PER_BLOCK`]).
    dropped: u64,
}

/// Controller/host half ([`EngineHandle::take_hw_io`]).
pub struct HwIoIo {
    /// MIDI for the hardware, oldest first.
    pub midi: Consumer<HwMidiEvent>,
}

/// Handle-side state kept in `EngineHandle`.
pub(crate) struct HwIoHandle {
    io: Option<HwIoIo>,
    requests: Producer<NodeKey>,
    results: Consumer<HwLatencyResult>,
}

/// Non-RT: allocate the rings (called by [`crate::create`]).
pub(crate) fn channel(sample_rate: u32) -> (HwIo, HwIoHandle) {
    let (midi_tx, midi_rx) = RingBuffer::new(HW_MIDI_CAPACITY);
    let (req_tx, req_rx) = RingBuffer::new(MEASURE_QUEUE);
    let (res_tx, res_rx) = RingBuffer::new(MEASURE_QUEUE);
    let sr = u64::from(sample_rate.max(1));
    (
        HwIo {
            rt: None,
            midi: midi_tx,
            requests: req_rx,
            results: res_tx,
            measure: None,
            note_off: None,
            timeout: sr * MEASURE_TIMEOUT_MS / 1000,
            note_len: sr * MEASURE_NOTE_MS / 1000,
            dropped: 0,
        },
        HwIoHandle {
            io: Some(HwIoIo { midi: midi_rx }),
            requests: req_tx,
            results: res_rx,
        },
    )
}

impl HwIo {
    /// RT (snapshot swap): install the new snapshot's state; returns the old one for the GC.
    pub(crate) fn swap(&mut self, rt: Option<Box<HwIoRt>>) -> Option<Box<HwIoRt>> {
        std::mem::replace(&mut self.rt, rt)
    }

    /// RT: the view the track jobs use for this sub-block.
    pub(crate) fn table(&mut self) -> HwIoTable {
        self.rt
            .as_mut()
            .map_or_else(HwIoTable::default, |rt| rt.table())
    }

    /// Messages dropped so far (diagnostics).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// RT, before the track jobs of a sub-block starting at engine sample `t`: gather the
    /// returns and advance a running measurement.
    pub(crate) fn gather_returns(&mut self, inputs: &[&[f32]], off: usize, n: usize, t: u64) {
        if self.measure.is_none()
            && let Ok(node) = self.requests.pop()
        {
            let instrument = self
                .rt
                .as_mut()
                .and_then(|rt| rt.entry_mut(node))
                .is_some_and(|e| !e.has_send());
            self.measure = Some(Measure {
                node,
                phase: Phase::Pending { waited: 0 },
                note_on: false,
                instrument,
            });
        }
        let Some(rt) = self.rt.as_mut() else {
            if let Some(m) = &mut self.measure
                && let Phase::Pending { waited } = &mut m.phase
            {
                *waited += n as u64;
                if *waited > self.timeout {
                    let node = m.node;
                    self.finish(node, None);
                }
            }
            return;
        };
        rt.gather_returns_at(inputs, off, n);
        let Some(m) = self.measure else {
            return;
        };
        let entry = rt.entry_mut(m.node);
        let mut done = None;
        let mut phase = m.phase;
        match (&mut phase, entry) {
            (Phase::Pending { waited }, None) => {
                *waited += n as u64;
                if *waited > self.timeout {
                    done = Some(None);
                }
            }
            (Phase::Pending { .. }, Some(_)) => {}
            (Phase::Sent { .. } | Phase::Peak { .. }, None) => done = Some(None),
            (Phase::Sent { t0 } | Phase::Peak { t0, .. }, Some(e)) => {
                let t0 = *t0;
                let [l, r] = e.returns(n);
                for i in 0..l.len() {
                    let frame = t + i as u64;
                    if frame < t0 {
                        continue;
                    }
                    let level = l[i].abs().max(r[i].abs());
                    match &mut phase {
                        Phase::Sent { .. } if level > MEASURE_THRESHOLD => {
                            if m.instrument {
                                done = Some(Some(frame - t0));
                                break;
                            }
                            phase = Phase::Peak {
                                t0,
                                at: frame,
                                level,
                                until: frame + MEASURE_PEAK_WINDOW,
                            };
                        }
                        Phase::Peak {
                            at,
                            level: best,
                            until,
                            ..
                        } => {
                            if frame >= *until {
                                done = Some(Some(*at - t0));
                                break;
                            }
                            if level > *best {
                                *best = level;
                                *at = frame;
                            }
                        }
                        _ => {}
                    }
                }
                if done.is_none() {
                    let end = t + n as u64;
                    if let Phase::Peak { at, until, .. } = phase
                        && end >= until
                    {
                        done = Some(Some(at - t0));
                    } else if end.saturating_sub(t0) > self.timeout {
                        done = Some(None);
                    }
                }
            }
        }
        if let Some(m) = &mut self.measure {
            m.phase = phase;
        }
        if let Some(result) = done {
            let samples = result.map(|s| s.min(u64::from(u32::MAX)) as u32);
            self.finish(m.node, samples);
        }
    }

    /// RT: end the measurement (an instrument's note is released at the next
    /// [`HwIo::write_sends`]) and report.
    fn finish(&mut self, node: NodeKey, samples: Option<u32>) {
        if let Some(m) = self.measure.take()
            && m.note_on
        {
            self.note_off = Some(node);
        }
        let _ = self.results.push(HwLatencyResult { node, samples });
    }

    /// RT, after the track jobs and the master bus of a sub-block starting at engine sample
    /// `t`: run the measurement's click / note, add the sends to the hardware outputs
    /// `[off, off + n)` and flush the staged MIDI to the host ring.
    pub(crate) fn write_sends(&mut self, outputs: &mut [&mut [f32]], off: usize, n: usize, t: u64) {
        let Some(rt) = self.rt.as_mut() else {
            return;
        };
        let mut failed = None;
        if let Some(m) = &mut self.measure
            && let Some(e) = rt.entry_mut(m.node)
        {
            let ch = e.routing.midi_channel.clamp(1, 16) - 1;
            match m.phase {
                Phase::Pending { .. } => {
                    m.instrument = !e.has_send();
                    if e.has_send() {
                        let [l, r] = e.send_mut(n);
                        l.fill(0.0);
                        r.fill(0.0);
                        if let (Some(a), Some(b)) = (l.first_mut(), r.first_mut()) {
                            *a = MEASURE_CLICK;
                            *b = MEASURE_CLICK;
                        }
                        m.phase = Phase::Sent { t0: t };
                    } else if e.routing.midi_out.is_some() {
                        e.stage(t, [0x90 | ch, MEASURE_KEY, MEASURE_VELOCITY]);
                        m.note_on = true;
                        m.phase = Phase::Sent { t0: t };
                    } else {
                        failed = Some(m.node);
                    }
                }
                Phase::Sent { t0 } | Phase::Peak { t0, .. } => {
                    if e.has_send() {
                        // The program stays off the send while measuring.
                        let [l, r] = e.send_mut(n);
                        l.fill(0.0);
                        r.fill(0.0);
                    }
                    if m.note_on && t + n as u64 >= t0 + self.note_len {
                        e.stage(t, [0x80 | ch, MEASURE_KEY, 0]);
                        m.note_on = false;
                    }
                }
            }
        }
        if let Some(node) = failed {
            self.measure = None;
            let _ = self.results.push(HwLatencyResult {
                node,
                samples: None,
            });
        }
        if let Some(node) = self.note_off.take()
            && let Some(e) = rt.entry_mut(node)
        {
            let ch = e.routing.midi_channel.clamp(1, 16) - 1;
            e.stage(t, [0x80 | ch, MEASURE_KEY, 0]);
        }
        rt.write_sends_at(outputs, off, n);
        let midi = &mut self.midi;
        let mut dropped = 0;
        if rt.drain_midi(|m| {
            if midi.push(m).is_err() {
                dropped += 1;
            }
        }) {
            dropped += 1;
        }
        self.dropped += dropped;
    }
}

impl EngineHandle {
    /// Take the host half of the hardware I/O (the MIDI ring) once; hosts without hardware
    /// MIDI out (web) never take it, and the engine then drops messages when the ring is
    /// full (nothing allocates).
    pub fn take_hw_io(&mut self) -> Option<HwIoIo> {
        self.hw_io.io.take()
    }

    /// Start measuring the round trip of external device `node` (`External::
    /// MeasureLatency`); the result arrives through [`EngineHandle::poll_hw_latency`]. One
    /// measurement runs at a time (requests queue).
    pub fn measure_hw_latency(&mut self, node: NodeKey) -> Result<(), crate::EngineError> {
        self.hw_io
            .requests
            .push(node)
            .map_err(|_| crate::EngineError::QueueFull)
    }

    /// The next finished measurement, if any.
    pub fn poll_hw_latency(&mut self) -> Option<HwLatencyResult> {
        self.hw_io.results.pop().ok()
    }
}
