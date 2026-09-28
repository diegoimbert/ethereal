//! Engine-side recording (owned by the `recording` wave-3 node; see `docs/WAVE3.md`).
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
//!   every monitored MIDI track (`TrackDesc::monitor`: armed with monitoring `Auto`, or `In`).
//!   Events that arrive late play at offset 0.
//! - **records MIDI**: while playing and recording with an armed MIDI track, every live
//!   event is also queued as [`RecordedMidi`] with its song position.
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
use crate::event::{EventKind, ProcessEvent};
use crate::graph::TrackDesc;
use crate::mixer::TrackRt;
use crate::transport::TransportInfo;

/// Seconds of input the capture ring holds (the host drains it every few ms).
pub const CAPTURE_RING_SECONDS: usize = 2;
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
/// Note ids of live notes live in the upper half of the id space (clip notes count up from 0).
const LIVE_NOTE_ID: u32 = 0x8000_0000;

/// One captured sub-block of hardware input (and of armed track input taps).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaptureBlock {
    /// Engine sample time of the first frame.
    pub sample_time: u64,
    pub frames: u32,
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
}

/// A live MIDI event heard while recording, with where it was played on the timeline
/// (uncompensated: the host subtracts its latency).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedMidi {
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
    let capacity = (config.sample_rate as usize).max(1) * CAPTURE_RING_SECONDS * channels;
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

        // Live MIDI due in this sub-block.
        let record_midi = recording && descs.iter().any(|t| t.armed && t.kind == TrackKind::Midi);
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
                if desc.kind != TrackKind::Midi || !track.monitor {
                    continue;
                }
                if let Some(first) = track.chain.first_mut() {
                    first.pending.push(ProcessEvent {
                        offset: offset as u32,
                        kind,
                    });
                }
            }
            if record_midi {
                let _ = self.midi_out.push(RecordedMidi {
                    sample_time: info.sample_time + offset,
                    position: info.position + offset as f64 * info.beats_per_sample,
                    beats_per_sample: info.beats_per_sample,
                    data: ev.data,
                });
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
        let hardware = descs.iter().any(|t| t.armed && t.audio_input.is_some());
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
        if !hardware && n_taps == 0 {
            return;
        }
        if self.blocks.slots() == 0 || self.tap_info.slots() < n_taps {
            return;
        }
        let dropped = self.write_input(inputs, off, frames);
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

    /// RT. Queue the hardware input channels (interleaved); `true` if the ring was full.
    fn write_input(&mut self, inputs: &[&[f32]], off: usize, frames: usize) -> bool {
        let channels = self.channels;
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
    /// Interleaved channels per frame.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Next captured block; its interleaved samples are appended to `out`
    /// (`frames * channels`, silence for a dropped block). Its taps are then in
    /// [`Self::taps`] / [`Self::tap_samples`]. Non-RT.
    pub fn next_block(&mut self, out: &mut Vec<f32>) -> Option<CaptureBlock> {
        let block = self.blocks.pop().ok()?;
        let n = block.frames as usize * self.channels;
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
            monitor: false,
            armed: false,
            clips: vec![],
            automation: vec![],
            racks: Vec::new(),
        }
    }

    /// Logs the events it receives with the sample time of the block.
    struct Probe(Arc<Mutex<Vec<(u64, ProcessEvent)>>>);

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
        assert_eq!(buf.len(), 3 * BLOCK * io.capture.channels());
        let first = blocks[0];
        assert_eq!(first.sample_time, 2 * BLOCK as u64);
        assert!((first.position - 4.0).abs() < 1e-9);
        // 120 bpm: 2 beats per second.
        assert!((first.beats_per_sample - 2.0 / SR as f64).abs() < 1e-12);
        // The click is at sample 3*BLOCK+10, i.e. frame BLOCK+10 of the capture, both channels.
        let ch = io.capture.channels();
        let hot: Vec<usize> = buf
            .iter()
            .enumerate()
            .filter(|(_, s)| **s != 0.0)
            .map(|(i, _)| i / ch)
            .collect();
        assert_eq!(hot, vec![BLOCK + 10, BLOCK + 10]);
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
            })
            .unwrap();
        // A late event plays at offset 0.
        io.midi_in
            .push(LiveMidi {
                sample_time: 3,
                data: [0x80, 61, 0],
            })
            .unwrap();
        // A future event waits for its block.
        io.midi_in
            .push(LiveMidi {
                sample_time: t + BLOCK as u64 + 5,
                data: [0x80, 60, 0],
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
}
