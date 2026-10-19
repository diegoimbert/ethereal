//! Engine-side recording.
//!
//! The engine calls [`RecordingRt::process`] once per sub-block (before any track is
//! rendered) and [`RecordingRt::capture`] once the sub-block's track jobs finished. They:
//!
//! - **capture hardware input**: while the transport is playing and recording and at least
//!   one armed track has an audio input, the sub-block's input channels are queued
//!   (interleaved) together with a [`CaptureBlock`] header carrying the engine sample time
//!   and the song position of the first frame. The host drains them off the audio thread
//!   ([`CaptureReader`]) and streams takes to disk. The header lets the host place every
//!   frame on the timeline after latency compensation, across loop wraps and locates
//!   (sub-blocks are linear in time, see [`crate::ProcessContext`]).
//! - **capture track input taps** (`tap-recording`, resampling): every armed audio track
//!   whose input is another track (`TrackDesc::input_tap`) also queues its aligned tapped
//!   signal (`crate::bus_tap`, stereo) with its track id and its PDC input latency
//!   ([`TapCapture`]): the tapped frame rendered at engine sample `k` belongs to the timeline
//!   position the engine rendered at `k - latency` (the alignment the monitor path hears).
//!   Up to [`MAX_TAP_CAPTURES`] tracks per block; recorded whether monitored or not.
//! - **plays live MIDI**: host-stamped [`LiveMidi`] events (engine sample time) are
//!   delivered, sample-accurately within the block they fall into, to the first device of
//!   every monitored MIDI track (`TrackDesc::monitor`: armed with monitoring `Auto`, or `In`)
//!   whose MIDI input filter accepts it (`TrackDesc::midi_input`: port and channel, see
//!   [`MidiInputFilter`]). Events that arrive late play at offset 0.
//! - **plays preview notes** (base-150): short messages queued for one track by
//!   [`crate::EngineHandle::preview_note`] (piano-roll note preview) go to that MIDI track's
//!   first device at the start of the next sub-block, whether it is monitored or not. They
//!   are never recorded.
//! - **records MIDI**: while playing and recording, every live event is also queued as
//!   [`RecordedMidi`] with its song position, once per armed MIDI track whose filter
//!   accepts it.
//! - **publishes the engine clock**: the sample time at which the next block starts, so the
//!   host can stamp MIDI input (see [`RecordingIo::clock`]).
//!
//! Everything on the audio thread is lock-free (`rtrb`) and allocation-free; the rings are
//! allocated in [`channel`], called by [`crate::create`]. The host half ([`RecordingIo`]) is
//! taken once with [`crate::EngineHandle::take_recording_io`]. Hosts without input (web)
//! never take it: nothing is captured then (the engine only captures while recording).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ether_protocol::model::{TrackId, TrackKind};
use rtrb::{Consumer, Producer, RingBuffer};

/// Re-exported so hosts can hold the ring halves (and build their own lock-free rings).
pub use rtrb;

use crate::config::EngineConfig;
use crate::engine::EngineHandle;
use crate::event::EventKind;
use crate::graph::TrackDesc;
use crate::mixer::TrackRt;
use crate::transport::TransportInfo;

/// Seconds of input the capture ring holds (the host drains it every few ms) when at most
/// [`CAPTURE_RING_CHANNELS`] channels are captured; proportionally less with more.
pub const CAPTURE_RING_SECONDS: usize = 2;
/// Channels the capture ring is sized for. An engine with more input channels
/// (`EngineConfig::input_channels`, e.g. a 32-channel interface) shares the same ring: a
/// block only carries the channels armed tracks record ([`CaptureBlock::channels`]).
pub const CAPTURE_RING_CHANNELS: usize = 8;
/// Capture headers the ring holds (sub-blocks can be short around loop points).
pub const CAPTURE_HEADERS: usize = 8192;
/// Live MIDI events queued towards the audio thread.
pub const LIVE_MIDI_CAPACITY: usize = 1024;
/// Recorded MIDI events queued towards the host.
pub const RECORDED_MIDI_CAPACITY: usize = 8192;
/// Armed tap tracks captured per sub-block (more are ignored).
pub const MAX_TAP_CAPTURES: usize = 8;
/// Stereo tap tracks the tap sample ring holds [`CAPTURE_RING_SECONDS`] of.
pub const TAP_RING_TRACKS: usize = 2;
/// [`TapCapture`] entries queued towards the host.
pub const TAP_INFOS: usize = 4096;
/// Preview notes ([`crate::EngineHandle::preview_note`]) queued between two sub-blocks.
pub const PREVIEW_NOTE_CAPACITY: usize = 64;
/// Note ids of live notes live in the upper half of the id space (clip notes count up from 0).
const LIVE_NOTE_ID: u32 = 0x8000_0000;
/// Live notes remembered per track ([`LiveNotes`]).
pub const MAX_LIVE_NOTES: usize = 128;
/// Sustain pedal (CC 64).
const SUSTAIN: u8 = 64;

/// The live notes and sustain pedals a track received and still holds (#559). Live MIDI
/// reaches a track only while it monitors (and while its input accepts the message); when
/// that changes between a key press and its release, the track would never get the
/// note-off. So a note-off (or pedal-up) of something the track holds is always delivered,
/// and a track that stops monitoring releases everything it holds at once.
#[derive(Debug, Default)]
pub(crate) struct LiveNotes {
    held: crate::held_notes::HeldNotes<()>,
    /// Channels whose sustain pedal is down (bit = channel).
    pedal: u16,
}

impl LiveNotes {
    /// Non-RT: allocates.
    pub(crate) fn new() -> Self {
        Self {
            held: crate::held_notes::HeldNotes::with_capacity(MAX_LIVE_NOTES),
            pedal: 0,
        }
    }

    /// RT. A live message delivered to the track: remember or forget what it holds.
    fn delivered(&mut self, data: [u8; 3], kind: &EventKind) {
        match *kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                ..
            } => self.held.on(note_id, channel, key, ()),
            EventKind::NoteOff { .. } => {
                self.ends(data, kind);
            }
            EventKind::Midi { .. } if data[0] & 0xf0 == 0xb0 && data[1] == SUSTAIN => {
                let bit = 1 << (data[0] & 0x0f);
                if data[2] >= 64 {
                    self.pedal |= bit;
                } else {
                    self.pedal &= !bit;
                }
            }
            _ => {}
        }
    }

    /// RT. Whether `kind` ends a note or pedal the track holds (forgotten then).
    fn ends(&mut self, data: [u8; 3], kind: &EventKind) -> bool {
        match *kind {
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                ..
            } => self.held.off(note_id, channel, key, |_| {}),
            EventKind::Midi { .. }
                if data[0] & 0xf0 == 0xb0 && data[1] == SUSTAIN && data[2] < 64 =>
            {
                let bit = 1 << (data[0] & 0x0f);
                let held = self.pedal & bit != 0;
                self.pedal &= !bit;
                held
            }
            _ => false,
        }
    }

    /// RT. Release everything held: `f` gets each note-off and pedal-up as raw MIDI.
    fn release(&mut self, mut f: impl FnMut([u8; 3])) {
        self.held.drain(|n| f([0x80 | n.channel, n.key, 0]));
        for ch in 0..16u8 {
            if self.pedal & (1 << ch) != 0 {
                f([0xb0 | ch, SUSTAIN, 0]);
            }
        }
        self.pedal = 0;
    }

    fn is_empty(&self) -> bool {
        self.held.is_empty() && self.pedal == 0
    }
}

/// One captured sub-block of hardware input (and of armed track input taps).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaptureBlock {
    /// Engine sample time of the first frame.
    pub sample_time: u64,
    pub frames: u32,
    /// Interleaved hardware input channels of this block: hardware channels
    /// `0..channels`, up to the highest one an armed track records (`0` = none captured,
    /// only taps).
    pub channels: u16,
    /// Song position (beats) of the first frame; linear over the block.
    pub position: f64,
    pub beats_per_sample: f64,
    /// The sample ring was full: this block's audio is missing (the reader yields silence).
    pub dropped: bool,
    /// Armed track input taps captured with this block ([`CaptureReader::taps`]).
    pub taps: u8,
    /// The tap sample ring was full: the taps' audio is missing (the reader yields silence).
    pub taps_dropped: bool,
}

/// One track input tap captured with a [`CaptureBlock`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapCapture {
    /// The armed consumer track (its input is `TrackInput::Track`).
    pub track: TrackId,
    /// The consumer's PDC input latency (samples): a frame captured at engine sample `k`
    /// belongs to the timeline position rendered at `k - latency`.
    pub latency: u32,
}

impl CaptureBlock {
    /// Song position of frame `i` of this block.
    pub fn position_at(&self, i: u64) -> f64 {
        self.position + i as f64 * self.beats_per_sample
    }
}

/// A short MIDI message for the audio thread, stamped in engine samples
/// (`sample_time` below the current block = play as soon as possible).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveMidi {
    pub sample_time: u64,
    pub data: [u8; 3],
    /// The input port ([`port_key`] of its name; [`UNTAGGED_PORT`] when unknown).
    pub port: u64,
}

/// [`LiveMidi::port`] of a message from no known port: only tracks listening to all ports
/// hear it.
pub const UNTAGGED_PORT: u64 = 0;

/// The key of a MIDI input port name, as carried by [`LiveMidi::port`] and
/// [`MidiInputFilter::port`] (64-bit FNV-1a; never [`UNTAGGED_PORT`]). Hosts compute it
/// once per port, when it is connected, so the MIDI callback neither hashes nor allocates.
pub const fn port_key(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    if h == UNTAGGED_PORT { 1 } else { h }
}

/// [`port_key`] of the computer MIDI keyboard (`Recording::LiveMidi`,
/// `ether_protocol::recording::COMPUTER_KEYBOARD_PORT`).
pub const COMPUTER_KEYBOARD_KEY: u64 = port_key(ether_protocol::recording::COMPUTER_KEYBOARD_PORT);

/// Which live MIDI a MIDI track monitors and records (`TrackInput::Midi`): all ports or
/// one ([`port_key`] of its name), all channels or one (0-based).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MidiInputFilter {
    pub port: Option<u64>,
    pub channel: Option<u8>,
}

impl MidiInputFilter {
    /// Does a message `data` from port `port` pass?
    pub fn accepts(&self, port: u64, data: [u8; 3]) -> bool {
        self.port.is_none_or(|p| p == port) && self.channel.is_none_or(|c| c == data[0] & 0x0f)
    }

    /// No filtering.
    pub fn is_all(&self) -> bool {
        self.port.is_none() && self.channel.is_none()
    }
}

/// A live MIDI event heard while recording, with where it was played on the timeline
/// (uncompensated: the host subtracts its latency).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedMidi {
    /// The armed MIDI track whose input filter accepted it (one event per such track).
    pub track: TrackId,
    pub sample_time: u64,
    pub position: f64,
    pub beats_per_sample: f64,
    pub data: [u8; 3],
}

/// Audio-thread half, owned by the [`crate::Engine`].
pub struct RecordingRt {
    blocks: Producer<CaptureBlock>,
    samples: Producer<f32>,
    channels: usize,
    midi_in: Consumer<LiveMidi>,
    midi_out: Producer<RecordedMidi>,
    clock: Arc<AtomicU64>,
    tap_info: Producer<TapCapture>,
    tap_samples: Producer<f32>,
    /// Preview notes for the next sub-block (capacity [`PREVIEW_NOTE_CAPACITY`], never
    /// grows: extra ones are dropped).
    previews: Vec<(TrackId, [u8; 3])>,
}

/// Host half: taken once from the [`EngineHandle`]. Its parts are meant for different
/// threads (disk writer, MIDI input callback, clock readers).
pub struct RecordingIo {
    pub capture: CaptureReader,
    /// Live MIDI towards the engine (MIDI input thread).
    pub midi_in: Producer<LiveMidi>,
    /// MIDI recorded by the engine (disk/take thread).
    pub midi_out: Consumer<RecordedMidi>,
    /// Engine sample time at which the next block starts (written by the audio thread).
    pub clock: Arc<AtomicU64>,
}

/// Reads [`CaptureBlock`]s and their interleaved samples (non-RT).
pub struct CaptureReader {
    blocks: Consumer<CaptureBlock>,
    samples: Consumer<f32>,
    channels: usize,
    tap_info: Consumer<TapCapture>,
    tap_samples: Consumer<f32>,
    /// Taps of the last block read and their samples (tap-major, interleaved stereo).
    taps: Vec<TapCapture>,
    tap_buf: Vec<f32>,
}

/// Allocate the recording rings for an engine (non-RT; called by [`crate::create`]).
pub fn channel(config: &EngineConfig) -> (RecordingRt, RecordingIo) {
    let channels = config.input_channels.max(1) as usize;
    let capacity = ((config.sample_rate as usize).max(1)
        * CAPTURE_RING_SECONDS
        * channels.min(CAPTURE_RING_CHANNELS))
    .max(config.max_block_size * channels);
    let (blocks_tx, blocks_rx) = RingBuffer::new(CAPTURE_HEADERS);
    let (samples_tx, samples_rx) = RingBuffer::new(capacity);
    let (midi_in_tx, midi_in_rx) = RingBuffer::new(LIVE_MIDI_CAPACITY);
    let (midi_out_tx, midi_out_rx) = RingBuffer::new(RECORDED_MIDI_CAPACITY);
    let clock = Arc::new(AtomicU64::new(0));
    let (tap_info_tx, tap_info_rx) = RingBuffer::new(TAP_INFOS);
    let (tap_samples_tx, tap_samples_rx) = RingBuffer::new(
        (config.sample_rate as usize).max(1) * CAPTURE_RING_SECONDS * 2 * TAP_RING_TRACKS,
    );
    (
        RecordingRt {
            blocks: blocks_tx,
            samples: samples_tx,
            channels,
            midi_in: midi_in_rx,
            midi_out: midi_out_tx,
            clock: clock.clone(),
            tap_info: tap_info_tx,
            tap_samples: tap_samples_tx,
            previews: Vec::with_capacity(PREVIEW_NOTE_CAPACITY),
        },
        RecordingIo {
            capture: CaptureReader {
                blocks: blocks_rx,
                samples: samples_rx,
                channels,
                tap_info: tap_info_rx,
                tap_samples: tap_samples_rx,
                taps: Vec::with_capacity(MAX_TAP_CAPTURES),
                tap_buf: Vec::new(),
            },
            midi_in: midi_in_tx,
            midi_out: midi_out_rx,
            clock,
        },
    )
}

impl EngineHandle {
    /// Take the host half of the recording rings (once; `None` afterwards).
    pub fn take_recording_io(&mut self) -> Option<RecordingIo> {
        self.recording.take()
    }
}

/// Hardware (left, right) channels of a track's `audio_input` (`(first, count)`, see
/// `TrackDesc::audio_input`): a mono input feeds both sides.
pub fn input_channels(audio_input: Option<(u16, u16)>) -> Option<(u16, u16)> {
    let (first, count) = audio_input?;
    (count > 0).then(|| (first, first + u16::from(count > 1)))
}

/// Decode a short MIDI message into an engine event (`None`: not playable, e.g. SysEx
/// fragments, clock).
pub fn midi_event(data: [u8; 3]) -> Option<EventKind> {
    let status = data[0];
    let channel = status & 0x0f;
    let key = data[1] & 0x7f;
    let velocity = f32::from(data[2] & 0x7f) / 127.0;
    let note_id = LIVE_NOTE_ID | (u32::from(channel) << 7) | u32::from(key);
    match status & 0xf0 {
        0x90 if data[2] > 0 => Some(EventKind::NoteOn {
            note_id,
            channel,
            key,
            velocity,
        }),
        0x80 | 0x90 => Some(EventKind::NoteOff {
            note_id,
            channel,
            key,
            velocity,
        }),
        0xa0..=0xe0 => Some(EventKind::Midi { data }),
        _ => None,
    }
}

impl RecordingRt {
    /// **RT.** Queue a preview note for `track` (played at the next [`Self::process`]).
    /// Dropped when [`PREVIEW_NOTE_CAPACITY`] are already waiting (no allocation).
    pub(crate) fn queue_preview(&mut self, track: TrackId, data: [u8; 3]) {
        if self.previews.len() < self.previews.capacity() {
            self.previews.push((track, data));
        }
    }

    /// **RT.** Called by the engine once per sub-block, before the tracks are rendered
    /// (live MIDI, engine clock). `descs[i]` and `tracks[i]` are the same track.
    pub(crate) fn process(
        &mut self,
        info: &TransportInfo,
        frames: usize,
        descs: &[TrackDesc],
        tracks: &mut [TrackRt],
    ) {
        let end = info.sample_time + frames as u64;
        let recording = info.playing && info.recording;

        // A track that stopped monitoring releases the live notes it holds (#559).
        for (desc, track) in descs.iter().zip(tracks.iter_mut()) {
            if track.monitor || track.live.is_empty() {
                continue;
            }
            let TrackRt {
                live,
                chain,
                expression,
                ..
            } = track;
            let Some(first) = chain.first_mut() else {
                live.release(|_| {});
                continue;
            };
            live.release(|data| {
                if let Some(kind) = midi_event(data) {
                    expression.live_input(&desc.expression, 0, kind, &mut first.pending);
                }
            });
        }

        // Live MIDI due in this sub-block.
        while let Ok(ev) = self.midi_in.peek() {
            if ev.sample_time >= end {
                break;
            }
            let Ok(ev) = self.midi_in.pop() else { break };
            let offset = ev
                .sample_time
                .saturating_sub(info.sample_time)
                .min(frames.saturating_sub(1) as u64);
            let Some(kind) = midi_event(ev.data) else {
                continue;
            };
            for (desc, track) in descs.iter().zip(tracks.iter_mut()) {
                if desc.kind != TrackKind::Midi {
                    continue;
                }
                let accepted = desc.midi_input.accepts(ev.port, ev.data);
                if accepted && recording && desc.armed {
                    let _ = self.midi_out.push(RecordedMidi {
                        track: desc.id,
                        sample_time: info.sample_time + offset,
                        position: info.position + offset as f64 * info.beats_per_sample,
                        beats_per_sample: info.beats_per_sample,
                        data: ev.data,
                    });
                }
                // Monitored input, or the end of a note the track holds (#559).
                if accepted && track.monitor {
                    track.live.delivered(ev.data, &kind);
                } else if !track.live.ends(ev.data, &kind) {
                    continue;
                }
                if let Some(first) = track.chain.first_mut() {
                    // v0.3 (`mpe`): read as MPE on tracks with MPE settings; unchanged
                    // otherwise (`ExpressionRt::live_input`).
                    track.expression.live_input(
                        &desc.expression,
                        offset as u32,
                        kind,
                        &mut first.pending,
                    );
                }
            }
        }

        // Preview notes: the target track only, monitored or not; never recorded.
        for (id, data) in self.previews.drain(..) {
            let Some(kind) = midi_event(data) else {
                continue;
            };
            let Some(i) = descs
                .iter()
                .position(|d| d.id == id && d.kind == TrackKind::Midi)
            else {
                continue;
            };
            let track = &mut tracks[i];
            if let Some(first) = track.chain.first_mut() {
                track
                    .expression
                    .live_input(&descs[i].expression, 0, kind, &mut first.pending);
            }
        }

        self.clock.store(end, Ordering::Release);
    }

    /// **RT.** Called by the engine once per sub-block, after the track jobs (the taps are
    /// gathered then): while recording, queue the hardware input of armed audio tracks and
    /// the aligned input taps of armed tap tracks.
    pub(crate) fn capture(
        &mut self,
        info: &TransportInfo,
        inputs: &[&[f32]],
        off: usize,
        frames: usize,
        descs: &[TrackDesc],
        tracks: &[TrackRt],
    ) {
        if !(info.playing && info.recording) || frames == 0 {
            return;
        }
        // Hardware channels `0..hardware`: up to the highest one an armed track records.
        let hardware = descs
            .iter()
            .filter(|t| t.armed)
            .filter_map(|t| crate::recording::input_channels(t.audio_input))
            .map(|(_, r)| usize::from(r) + 1)
            .max()
            .unwrap_or(0)
            .min(self.channels);
        let mut taps = [0usize; MAX_TAP_CAPTURES];
        let mut n_taps = 0;
        for (i, (desc, track)) in descs.iter().zip(tracks).enumerate() {
            if n_taps < MAX_TAP_CAPTURES
                && desc.armed
                && desc.kind == TrackKind::Audio
                && desc.input_tap.is_some()
                && track.input_tap.signal(frames).is_some()
            {
                taps[n_taps] = i;
                n_taps += 1;
            }
        }
        if hardware == 0 && n_taps == 0 {
            return;
        }
        if self.blocks.slots() == 0 || self.tap_info.slots() < n_taps {
            return;
        }
        let dropped = self.write_input(inputs, off, frames, hardware);
        let taps_dropped = n_taps > 0 && self.write_taps(&taps[..n_taps], frames, tracks);
        for &ti in &taps[..n_taps] {
            let _ = self.tap_info.push(TapCapture {
                track: descs[ti].id,
                latency: crate::bus_tap::input_latency(tracks, ti),
            });
        }
        let _ = self.blocks.push(CaptureBlock {
            sample_time: info.sample_time,
            frames: frames as u32,
            channels: hardware as u16,
            position: info.position,
            beats_per_sample: info.beats_per_sample,
            dropped,
            taps: n_taps as u8,
            taps_dropped,
        });
    }

    /// RT. Queue the aligned taps of `taps` (tap-major, interleaved stereo); `true` if the
    /// ring was full.
    fn write_taps(&mut self, taps: &[usize], frames: usize, tracks: &[TrackRt]) -> bool {
        let Ok(mut chunk) = self.tap_samples.write_chunk(taps.len() * frames * 2) else {
            return true;
        };
        let (a, b) = chunk.as_mut_slices();
        let mut i = 0;
        for &ti in taps {
            let signal = tracks[ti].input_tap.signal(frames);
            for f in 0..frames {
                for ch in 0..2 {
                    let s = signal.map_or(0.0, |sig| sig[ch][f]);
                    if i < a.len() {
                        a[i] = s;
                    } else {
                        b[i - a.len()] = s;
                    }
                    i += 1;
                }
            }
        }
        chunk.commit_all();
        false
    }

    /// RT. Queue hardware input channels `0..channels` (interleaved); `true` if the ring
    /// was full.
    fn write_input(
        &mut self,
        inputs: &[&[f32]],
        off: usize,
        frames: usize,
        channels: usize,
    ) -> bool {
        if channels == 0 {
            return false;
        }
        match self.samples.write_chunk(frames * channels) {
            Ok(mut chunk) => {
                let (a, b) = chunk.as_mut_slices();
                let mut i = 0;
                for f in 0..frames {
                    for ch in 0..channels {
                        let s = inputs
                            .get(ch)
                            .and_then(|input| input.get(off + f))
                            .copied()
                            .unwrap_or(0.0);
                        if i < a.len() {
                            a[i] = s;
                        } else {
                            b[i - a.len()] = s;
                        }
                        i += 1;
                    }
                }
                chunk.commit_all();
                false
            }
            Err(_) => true,
        }
    }
}

impl CaptureReader {
    /// Most interleaved channels per frame (`EngineConfig::input_channels`); each block
    /// carries [`CaptureBlock::channels`] of them.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Next captured block; its interleaved samples are appended to `out`
    /// (`frames * block.channels`, silence for a dropped block). Its taps are then in
    /// [`Self::taps`] / [`Self::tap_samples`]. Non-RT.
    pub fn next_block(&mut self, out: &mut Vec<f32>) -> Option<CaptureBlock> {
        let block = self.blocks.pop().ok()?;
        let n = block.frames as usize * usize::from(block.channels);
        self.read_taps(&block);
        if block.dropped {
            out.resize(out.len() + n, 0.0);
            return Some(block);
        }
        Self::read(&mut self.samples, n, out);
        Some(block)
    }

    /// The track input taps captured with the last block read.
    pub fn taps(&self) -> &[TapCapture] {
        &self.taps
    }

    /// Stereo samples of tap `i` of the last block read (`frames * 2`, interleaved).
    pub fn tap_samples(&self, i: usize) -> &[f32] {
        let n = self.tap_buf.len() / self.taps.len().max(1);
        self.tap_buf.get(i * n..(i + 1) * n).unwrap_or(&[])
    }

    /// The stereo samples of every tap of the last block read, tap after tap.
    pub fn tap_buffer(&self) -> &[f32] {
        &self.tap_buf
    }

    fn read_taps(&mut self, block: &CaptureBlock) {
        self.taps.clear();
        self.tap_buf.clear();
        for _ in 0..block.taps {
            match self.tap_info.pop() {
                Ok(t) => self.taps.push(t),
                Err(_) => break,
            }
        }
        let n = usize::from(block.taps) * block.frames as usize * 2;
        if block.taps_dropped {
            self.tap_buf.resize(n, 0.0);
        } else {
            Self::read(&mut self.tap_samples, n, &mut self.tap_buf);
        }
        // Stay consistent even if the info ring was short (cannot happen: checked on push).
        self.tap_buf
            .resize(self.taps.len() * block.frames as usize * 2, 0.0);
    }

    fn read(ring: &mut Consumer<f32>, n: usize, out: &mut Vec<f32>) {
        if n == 0 {
            return;
        }
        match ring.read_chunk(n) {
            Ok(chunk) => {
                let (a, b) = chunk.as_slices();
                out.extend_from_slice(a);
                out.extend_from_slice(b);
                chunk.commit_all();
            }
            // Cannot happen (samples are committed before their header); stay aligned.
            Err(_) => out.resize(out.len() + n, 0.0),
        }
    }

    /// Drop everything queued (before a new take).
    pub fn clear(&mut self) {
        let mut scratch = Vec::new();
        while self.next_block(&mut scratch).is_some() {
            scratch.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ChainEntry, RenderGraphDesc};
    use crate::{AudioBuffers, Node, PrepareConfig, ProcessContext, ProcessStatus};
    use crate::{TransportControl, create};
    use ether_protocol::model::{Beats, TrackId, Ulid};
    use std::sync::Mutex;

    const SR: u32 = 48_000;
    const BLOCK: usize = 256;

    fn track(id: u128, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
        TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: TrackId(Ulid(id)),
            kind,
            chain: vec![],
            output,
            group: None,
            sends: vec![],
            volume: 1.0,
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
        }
    }

    /// Logs the events it receives with the sample time of the block.
    struct Probe(Arc<Mutex<Vec<(u64, crate::event::ProcessEvent)>>>);

    impl Node for Probe {
        fn prepare(&mut self, _: &PrepareConfig) {}
        fn reset(&mut self) {}
        fn process(&mut self, ctx: &mut ProcessContext<'_>, _: &mut AudioBuffers) -> ProcessStatus {
            if let Ok(mut log) = self.0.try_lock() {
                for e in ctx.events {
                    log.push((ctx.transport.sample_time, *e));
                }
            }
            ProcessStatus::Silent
        }
    }

    fn engine() -> crate::EngineParts {
        create(crate::EngineConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_nodes: 16,
            ..Default::default()
        })
    }

    fn run(engine: &mut crate::Engine, blocks: usize, input: &dyn Fn(u64) -> f32, t: &mut u64) {
        let mut l = vec![0.0; BLOCK];
        let mut r = vec![0.0; BLOCK];
        let mut il = vec![0.0; BLOCK];
        for _ in 0..blocks {
            for (i, s) in il.iter_mut().enumerate() {
                *s = input(*t + i as u64);
            }
            let ir = il.clone();
            engine.process(&[&il, &ir], &mut [&mut l, &mut r], BLOCK);
            *t += BLOCK as u64;
        }
    }

    #[test]
    fn input_channels_from_first_and_count() {
        assert_eq!(input_channels(Some((0, 2))), Some((0, 1)));
        assert_eq!(input_channels(Some((3, 1))), Some((3, 3)));
        assert_eq!(input_channels(Some((0, 0))), None);
        assert_eq!(input_channels(None), None);
    }

    #[test]
    fn midi_decoding() {
        assert!(matches!(
            midi_event([0x91, 60, 127]),
            Some(EventKind::NoteOn {
                channel: 1,
                key: 60,
                ..
            })
        ));
        assert!(matches!(
            midi_event([0x90, 60, 0]),
            Some(EventKind::NoteOff { key: 60, .. })
        ));
        assert!(matches!(
            midi_event([0xb0, 1, 2]),
            Some(EventKind::Midi { .. })
        ));
        assert_eq!(midi_event([0xf8, 0, 0]), None);
        // Note on/off of the same key share an id.
        let id = |e| match e {
            Some(EventKind::NoteOn { note_id, .. } | EventKind::NoteOff { note_id, .. }) => note_id,
            _ => 0,
        };
        assert_eq!(id(midi_event([0x90, 64, 1])), id(midi_event([0x80, 64, 0])));
    }

    #[test]
    fn captures_input_only_while_recording_with_positions() {
        let mut parts = engine();
        let mut io = parts.handle.take_recording_io().unwrap();
        assert!(parts.handle.take_recording_io().is_none());
        let mut master = track(1, TrackKind::Master, None);
        master.armed = false;
        let mut audio = track(2, TrackKind::Audio, Some(master.id));
        audio.armed = true;
        audio.audio_input = Some((0, 1));
        parts
            .handle
            .publish(RenderGraphDesc {
                tracks: vec![master, audio],
                ..Default::default()
            })
            .unwrap();
        let mut t = 0;
        let click = |s: u64| if s == 3 * BLOCK as u64 + 10 { 1.0 } else { 0.0 };
        run(&mut parts.engine, 2, &click, &mut t);
        let mut buf = Vec::new();
        assert!(
            io.capture.next_block(&mut buf).is_none(),
            "not recording yet"
        );

        parts
            .handle
            .transport(TransportControl::Locate {
                position: Beats(4.0),
            })
            .unwrap();
        parts
            .handle
            .transport(TransportControl::SetRecording { enabled: true })
            .unwrap();
        parts.handle.transport(TransportControl::Play).unwrap();
        run(&mut parts.engine, 3, &click, &mut t);
        assert_eq!(io.clock.load(Ordering::Acquire), t);

        let mut blocks = Vec::new();
        while let Some(b) = io.capture.next_block(&mut buf) {
            blocks.push(b);
        }
        assert_eq!(blocks.len(), 3);
        let first = blocks[0];
        // Only the channels the armed track records: input 1 (mono).
        assert_eq!(first.channels, 1);
        assert_eq!(buf.len(), 3 * BLOCK);
        assert_eq!(first.sample_time, 2 * BLOCK as u64);
        assert!((first.position - 4.0).abs() < 1e-9);
        // 120 bpm: 2 beats per second.
        assert!((first.beats_per_sample - 2.0 / SR as f64).abs() < 1e-12);
        // The click is at sample 3*BLOCK+10, i.e. frame BLOCK+10 of the capture.
        let ch = usize::from(first.channels);
        let hot: Vec<usize> = buf
            .iter()
            .enumerate()
            .filter(|(_, s)| **s != 0.0)
            .map(|(i, _)| i / ch)
            .collect();
        assert_eq!(hot, vec![BLOCK + 10]);
        let b1 = blocks[1];
        assert!(
            (b1.position_at(10) - (4.0 + (BLOCK + 10) as f64 * first.beats_per_sample)).abs()
                < 1e-9
        );

        // Stop recording: nothing more is captured.
        parts
            .handle
            .transport(TransportControl::SetRecording { enabled: false })
            .unwrap();
        run(&mut parts.engine, 2, &click, &mut t);
        assert!(io.capture.next_block(&mut buf).is_none());
    }

    #[test]
    fn captures_up_to_the_highest_armed_channel_of_a_multichannel_input() {
        let mut parts = create(crate::EngineConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_nodes: 16,
            input_channels: 4,
            ..Default::default()
        });
        let mut io = parts.handle.take_recording_io().unwrap();
        assert_eq!(io.capture.channels(), 4);
        let master = track(1, TrackKind::Master, None);
        let mut audio = track(2, TrackKind::Audio, Some(master.id));
        audio.armed = true;
        // Inputs 3/4.
        audio.audio_input = Some((2, 2));
        parts
            .handle
            .publish(RenderGraphDesc {
                tracks: vec![master, audio],
                ..Default::default()
            })
            .unwrap();
        for c in [
            TransportControl::SetRecording { enabled: true },
            TransportControl::Play,
        ] {
            parts.handle.transport(c).unwrap();
        }
        // Channel `c` of the interface carries `c + 1` (a 5th one would be ignored).
        let ins: Vec<Vec<f32>> = (0..5).map(|c| vec![c as f32 + 1.0; BLOCK]).collect();
        let ins: Vec<&[f32]> = ins.iter().map(Vec::as_slice).collect();
        let (mut l, mut r) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
        for _ in 0..2 {
            parts.engine.process(&ins, &mut [&mut l, &mut r], BLOCK);
        }
        let mut buf = Vec::new();
        let mut n = 0;
        while let Some(b) = io.capture.next_block(&mut buf) {
            assert_eq!(b.channels, 4, "channels 1..=4: up to input 4");
            n += 1;
        }
        assert_eq!(n, 2);
        assert_eq!(buf.len(), 2 * BLOCK * 4);
        for frame in buf.as_chunks::<4>().0 {
            assert_eq!(*frame, [1.0, 2.0, 3.0, 4.0]);
        }
    }

    /// #433: a track filtered to port A / channel 1 ignores port B and channel 2, for
    /// monitoring and recording; an unfiltered track hears everything.
    #[test]
    fn midi_input_filters_apply_to_monitoring_and_recording() {
        let mut parts = engine();
        let mut io = parts.handle.take_recording_io().unwrap();
        let (port_a, port_b) = (port_key("Port A"), port_key("Port B"));
        assert_ne!(port_a, port_b);
        let master = track(1, TrackKind::Master, None);
        let mut tracks = vec![master.clone()];
        let mut logs = Vec::new();
        for (id, filter) in [
            (
                2,
                MidiInputFilter {
                    port: Some(port_a),
                    channel: Some(0),
                },
            ),
            (3, MidiInputFilter::default()),
        ] {
            let log = Arc::new(Mutex::new(Vec::new()));
            let key = parts.handle.add_node(Box::new(Probe(log.clone()))).unwrap();
            let mut t = track(id, TrackKind::Midi, Some(master.id));
            t.armed = true;
            t.monitor = true;
            t.midi_input = filter;
            t.chain = vec![ChainEntry {
                node: key,
                enabled: true,
                sidechain: None,
            }];
            tracks.push(t);
            logs.push(log);
        }
        let (filtered, all) = (tracks[1].id, tracks[2].id);
        parts
            .handle
            .publish(RenderGraphDesc {
                tracks,
                ..Default::default()
            })
            .unwrap();
        for c in [
            TransportControl::SetRecording { enabled: true },
            TransportControl::Play,
        ] {
            parts.handle.transport(c).unwrap();
        }
        let mut t = 0;
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        let send = |io: &mut RecordingIo, port, data| {
            io.midi_in
                .push(LiveMidi {
                    sample_time: 0,
                    data,
                    port,
                })
                .unwrap();
        };
        // Port B channel 1, port A channel 2, the computer keyboard: not for the filtered track.
        send(&mut io, port_b, [0x90, 60, 100]);
        send(&mut io, port_a, [0x91, 61, 100]);
        send(&mut io, COMPUTER_KEYBOARD_KEY, [0x90, 62, 100]);
        // Port A channel 1: both.
        send(&mut io, port_a, [0x90, 63, 100]);
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        let keys = |log: &Arc<Mutex<Vec<(u64, crate::event::ProcessEvent)>>>| -> Vec<u8> {
            log.lock()
                .unwrap()
                .iter()
                .filter_map(|(_, e)| match e.kind {
                    EventKind::NoteOn { key, .. } => Some(key),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(keys(&logs[0]), vec![63], "monitored: port A channel 1 only");
        assert_eq!(keys(&logs[1]), vec![60, 61, 62, 63]);
        let mut recorded = Vec::new();
        while let Ok(r) = io.midi_out.pop() {
            recorded.push((r.track, r.data[1]));
        }
        assert_eq!(
            recorded,
            vec![(all, 60), (all, 61), (all, 62), (filtered, 63), (all, 63)],
            "recorded per track through its filter"
        );
    }

    #[test]
    fn live_midi_reaches_monitored_tracks_and_is_recorded() {
        let mut parts = engine();
        let mut io = parts.handle.take_recording_io().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let other = Arc::new(Mutex::new(Vec::new()));
        let key = parts.handle.add_node(Box::new(Probe(log.clone()))).unwrap();
        let key2 = parts
            .handle
            .add_node(Box::new(Probe(other.clone())))
            .unwrap();
        let master = track(1, TrackKind::Master, None);
        let mut midi = track(2, TrackKind::Midi, Some(master.id));
        midi.armed = true;
        midi.monitor = true;
        midi.chain = vec![ChainEntry {
            node: key,
            enabled: true,
            sidechain: None,
        }];
        let mut quiet = track(3, TrackKind::Midi, Some(master.id));
        quiet.chain = vec![ChainEntry {
            node: key2,
            enabled: true,
            sidechain: None,
        }];
        parts
            .handle
            .publish(RenderGraphDesc {
                tracks: vec![master, midi, quiet],
                ..Default::default()
            })
            .unwrap();
        let mut t = 0;
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);

        // Monitoring while stopped: an event due in the next block lands at its offset.
        io.midi_in
            .push(LiveMidi {
                sample_time: t + 100,
                data: [0x90, 60, 100],
                port: UNTAGGED_PORT,
            })
            .unwrap();
        // A late event plays at offset 0.
        io.midi_in
            .push(LiveMidi {
                sample_time: 3,
                data: [0x80, 61, 0],
                port: UNTAGGED_PORT,
            })
            .unwrap();
        // A future event waits for its block.
        io.midi_in
            .push(LiveMidi {
                sample_time: t + BLOCK as u64 + 5,
                data: [0x80, 60, 0],
                port: UNTAGGED_PORT,
            })
            .unwrap();
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        {
            let log = log.lock().unwrap();
            let offsets: Vec<u32> = log.iter().map(|(_, e)| e.offset).collect();
            assert_eq!(offsets, vec![0, 100], "{log:?}");
        }
        assert!(
            other.lock().unwrap().is_empty(),
            "unmonitored track stays silent"
        );
        assert!(
            io.midi_out.pop().is_err(),
            "not recording: nothing recorded"
        );
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        assert_eq!(log.lock().unwrap().last().unwrap().1.offset, 5);

        // Recording: events are recorded with their song position.
        parts
            .handle
            .transport(TransportControl::SetRecording { enabled: true })
            .unwrap();
        parts.handle.transport(TransportControl::Play).unwrap();
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        let start = t;
        io.midi_in
            .push(LiveMidi {
                sample_time: start + 48,
                data: [0x90, 64, 90],
                port: UNTAGGED_PORT,
            })
            .unwrap();
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        let rec = io.midi_out.pop().unwrap();
        assert_eq!(rec.sample_time, start + 48);
        assert_eq!(rec.data, [0x90, 64, 90]);
        let expected = (BLOCK + 48) as f64 * 2.0 / SR as f64;
        assert!(
            (rec.position - expected).abs() < 1e-9,
            "{rec:?} vs {expected}"
        );
    }

    /// base-150: a preview note reaches only its track's first device, monitored or not,
    /// and is never recorded; an audio track or an unknown track ignores it.
    #[test]
    fn preview_notes_reach_their_track_only_and_are_not_recorded() {
        let mut parts = engine();
        let mut io = parts.handle.take_recording_io().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let other = Arc::new(Mutex::new(Vec::new()));
        let key = parts.handle.add_node(Box::new(Probe(log.clone()))).unwrap();
        let key2 = parts
            .handle
            .add_node(Box::new(Probe(other.clone())))
            .unwrap();
        let master = track(1, TrackKind::Master, None);
        let mut target = track(2, TrackKind::Midi, Some(master.id));
        target.chain = vec![ChainEntry {
            node: key,
            enabled: true,
            sidechain: None,
        }];
        let mut armed = track(3, TrackKind::Midi, Some(master.id));
        armed.armed = true;
        armed.monitor = true;
        armed.chain = vec![ChainEntry {
            node: key2,
            enabled: true,
            sidechain: None,
        }];
        let target_id = target.id;
        parts
            .handle
            .publish(RenderGraphDesc {
                tracks: vec![master, target, armed],
                ..Default::default()
            })
            .unwrap();
        parts
            .handle
            .transport(TransportControl::SetRecording { enabled: true })
            .unwrap();
        parts.handle.transport(TransportControl::Play).unwrap();
        let mut t = 0;
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);

        parts
            .handle
            .preview_note(target_id, [0x90, 60, 100])
            .unwrap();
        parts
            .handle
            .preview_note(TrackId(Ulid(99)), [0x90, 61, 100])
            .unwrap();
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);
        parts.handle.preview_note(target_id, [0x80, 60, 0]).unwrap();
        run(&mut parts.engine, 1, &|_| 0.0, &mut t);

        let kinds: Vec<_> = log
            .lock()
            .unwrap()
            .iter()
            .map(|(_, e)| (e.offset, e.kind))
            .collect();
        assert!(
            matches!(
                kinds.as_slice(),
                [
                    (0, EventKind::NoteOn { key: 60, .. }),
                    (0, EventKind::NoteOff { key: 60, .. })
                ]
            ),
            "{kinds:?}"
        );
        assert!(
            other.lock().unwrap().is_empty(),
            "other tracks don't hear it"
        );
        assert!(io.midi_out.pop().is_err(), "previews are never recorded");
    }
}
