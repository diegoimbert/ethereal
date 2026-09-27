//! Native audio/MIDI input capture (owned by the `recording` wave-3 node; see
//! `docs/WAVE3.md`).
//!
//! ```text
//!  cpal input callback ──ring (stereo f32)──▶ InputFeed (output callback) ──▶ Engine::process(inputs)
//!  midir callbacks ──LiveMidi (engine-sample stamped)──▶ engine live MIDI ring ──▶ monitored MIDI tracks
//!  Engine (recording hook) ──CaptureBlock + samples, RecordedMidi──▶ writer thread ──▶ media/rec-*.wav
//! ```
//!
//! - **Input device**: `AudioSettings::input_device` (host-handled `Engine::SetAudioConfig`
//!   `input_device`). `None` opens no input (no microphone prompt). A device name opens that
//!   cpal input device at the engine sample rate, next to the output stream. The special
//!   name `loopback` / `loopback:<samples>` feeds the engine's own output back as input,
//!   delayed by `<samples>` (at least one engine block): a device-less round trip for tests
//!   and diagnostics, usable with the `null`/`offline` backends.
//! - **RT side**: the input callback only writes the ring; the output callback ([`InputFeed`],
//!   owned by `RtRenderer`) only reads it and publishes the (wall clock, engine sample) pair
//!   used to stamp MIDI. No locks, no allocation.
//! - **Writer thread** (one per engine, non-RT): drains the engine's capture ring, splits
//!   it into takes and streams them as 32-bit float WAV files into
//!   `<projects_root>/<project>/media/`. MIDI played while recording is collected there too.
//! - **MIDI input** (`midir`): every port is connected the first time the inputs are listed
//!   (the recording panel does at startup), so connecting needs no extra permission and
//!   costs nothing until recording features are used.
//!
//! # Latency compensation
//!
//! A performer plays along with what they *hear*. With
//! - `out` = output latency (cpal: `playback - callback` of the output stream),
//! - `in` = input latency (cpal: `callback - capture` of the input stream),
//! - `backlog` = frames waiting in the input ring when the output callback reads it
//!   (smoothed; the input and output callbacks are not synchronized),
//!
//! a sound played while hearing engine sample `h` reaches the engine as sample
//! `k = h + L`, with the round-trip latency **`L = in + out + backlog`** (samples). The
//! writer therefore places captured frame `k` at the song position the engine rendered for
//! sample `k - L` (looked up in the per-sub-block [`CaptureBlock`] headers, so loops,
//! locates and tempo changes are exact) and drops frames whose position is outside the
//! session's keep range (count-in, punch). A take ends at a timeline discontinuity (loop
//! wrap, locate) or when frames leave the keep range; each take becomes a clip starting at
//! the position of its first frame.
//!
//! Live MIDI is stamped `engine_sample(now) + block` (one engine block of scheduling delay
//! keeps sub-block offsets jitter-free); a recorded event is placed at
//! `position - (block + out) * beats_per_sample`.
//!
//! The loopback input has `in = delay`, `out = 0`, `backlog = 0`, so `L` is exact there
//! (see the tests).

mod cpal_input;
mod midi;
mod writer;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ether_controller::{BridgeError, RecordSession, RecordedTakes};
use ether_core::EngineHandle;
use ether_core::protocol::recording::{AudioInputChannel, InputList};
use ether_core::recording::rtrb::{Consumer, Producer, RingBuffer};
use ether_core::recording::{LiveMidi, RecordingIo};

use crate::audio::{AudioBackendKind, AudioSettings};
use crate::rt::AudioShared;

pub use cpal_input::list_input_devices;
pub(crate) use cpal_input::open_input;

/// Input channels fed to the engine (`EngineConfig::input_channels`).
pub const INPUT_CHANNELS: usize = 2;
/// Input ring capacity, in engine blocks.
const INPUT_RING_BLOCKS: usize = 16;
/// Name prefix of the loopback test input (`loopback` or `loopback:<samples>`).
pub const LOOPBACK: &str = "loopback";
/// Default loopback delay (samples).
const DEFAULT_LOOPBACK: usize = 1024;
/// How long `stop` waits for the engine to acknowledge the end of recording.
const STOP_WAIT: Duration = Duration::from_secs(1);

/// What the next [`InputFeed`] reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FeedMode {
    #[default]
    Silent,
    /// A cpal input stream (opened next to the output stream).
    Device,
    Loopback {
        delay: usize,
    },
}

/// Recording state shared by the audio threads, the controller thread (bridge), the writer
/// thread and MIDI callbacks. Lives in [`AudioShared`].
pub struct RecordingShared {
    epoch: Instant,
    /// Seqlock over (`clock_ns`, `clock_sample`): wall time (ns since `epoch`) at which the
    /// engine started rendering sample `clock_sample`.
    seq: AtomicU64,
    clock_ns: AtomicU64,
    clock_sample: AtomicU64,
    sample_rate: AtomicU32,
    /// Engine max block (MIDI scheduling delay).
    block: AtomicU32,
    input_latency: AtomicU32,
    output_latency: AtomicU32,
    /// Smoothed input ring backlog in frames (`f32` bits).
    backlog: AtomicU32,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    projects_root: Option<PathBuf>,
    mode: FeedMode,
    input_device: Option<String>,
    input_host: Option<String>,
    /// Channels of the opened input device.
    input_channels: u16,
    /// Producer of the current feed's ring, taken by the input stream.
    pending_input: Option<Producer<f32>>,
    engine_clock: Option<Arc<AtomicU64>>,
    midi_in: Option<Arc<Mutex<Producer<LiveMidi>>>>,
    midi: midi::MidiInputs,
    writer: Option<writer::WriterHandle>,
}

impl Default for RecordingShared {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            seq: AtomicU64::new(0),
            clock_ns: AtomicU64::new(0),
            clock_sample: AtomicU64::new(0),
            sample_rate: AtomicU32::new(48_000),
            block: AtomicU32::new(256),
            input_latency: AtomicU32::new(0),
            output_latency: AtomicU32::new(0),
            backlog: AtomicU32::new(0),
            inner: Mutex::new(Inner::default()),
        }
    }
}

impl std::fmt::Debug for RecordingShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingShared")
            .field("input_latency", &self.input_latency())
            .field(
                "output_latency",
                &self.output_latency.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl RecordingShared {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Root of the project store: takes are written to `<root>/<project>/media/`.
    pub fn set_projects_root(&self, root: PathBuf) {
        self.inner().projects_root = Some(root);
    }

    /// Input latency in samples (0 without an input).
    pub fn input_latency(&self) -> u32 {
        self.input_latency.load(Ordering::Relaxed)
    }

    /// Measured output latency in samples, or `fallback` if the backend reports none.
    pub fn output_latency_or(&self, fallback: u32) -> u32 {
        match self.output_latency.load(Ordering::Relaxed) {
            0 => fallback,
            n => n,
        }
    }

    /// Round-trip latency compensated on recorded audio: `in + out + backlog` (samples).
    pub fn round_trip_latency(&self) -> u32 {
        self.input_latency()
            + self.output_latency.load(Ordering::Relaxed)
            + f32::from_bits(self.backlog.load(Ordering::Relaxed)).round() as u32
    }

    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// **RT.** The engine is about to render `sample` now.
    fn publish_clock(&self, sample: u64) {
        let ns = self.now_ns();
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.clock_ns.store(ns, Ordering::Relaxed);
        self.clock_sample.store(sample, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    fn read_clock(&self) -> (u64, u64) {
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let ns = self.clock_ns.load(Ordering::Relaxed);
            let sample = self.clock_sample.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return (ns, sample);
            }
        }
    }

    /// Engine sample time at which a MIDI message received now should play: the current
    /// engine time plus one block of scheduling delay.
    fn stamp_now(&self) -> u64 {
        let (ns, sample) = self.read_clock();
        let sr = u64::from(self.sample_rate.load(Ordering::Relaxed));
        let elapsed = self.now_ns().saturating_sub(ns);
        sample + elapsed * sr / 1_000_000_000 + u64::from(self.block.load(Ordering::Relaxed))
    }

    /// Queue a live MIDI message for the engine (MIDI thread; non-RT).
    fn push_live(&self, data: [u8; 3]) -> bool {
        let stamp = self.stamp_now();
        let midi_in = self.inner().midi_in.clone();
        let Some(midi_in) = midi_in else {
            return false;
        };
        let mut p = midi_in.lock().unwrap_or_else(|p| p.into_inner());
        p.push(LiveMidi {
            sample_time: stamp,
            data,
        })
        .is_ok()
    }

    /// MIDI latency compensated on recorded notes: scheduling delay + output latency.
    fn midi_latency(&self) -> u32 {
        self.block.load(Ordering::Relaxed) + self.output_latency.load(Ordering::Relaxed)
    }
}

fn parse_loopback(name: &str) -> Option<usize> {
    let rest = name.strip_prefix(LOOPBACK)?;
    match rest.strip_prefix(':') {
        Some(n) => n.trim().parse().ok(),
        None if rest.is_empty() => Some(DEFAULT_LOOPBACK),
        None => None,
    }
}

/// Non-RT, before a backend starts: choose what the input feed reads from `settings`.
pub fn configure_input(shared: &AudioShared, settings: &AudioSettings) {
    let rec = &shared.recording;
    let mode = match settings.input_device.as_deref() {
        None | Some("") => FeedMode::Silent,
        Some(name) => match parse_loopback(name) {
            Some(delay) => FeedMode::Loopback { delay },
            None if settings.backend == AudioBackendKind::Cpal => FeedMode::Device,
            None => FeedMode::Silent,
        },
    };
    let mut inner = rec.inner();
    inner.mode = mode;
    inner.input_device = settings.input_device.clone();
    inner.input_host = settings.host.clone();
    inner.input_channels = 0;
    inner.pending_input = None;
    rec.input_latency.store(0, Ordering::Relaxed);
    rec.output_latency.store(0, Ordering::Relaxed);
    rec.backlog.store(0f32.to_bits(), Ordering::Relaxed);
}

/// **RT.** Record the output stream's latency (cpal output callback).
pub fn output_timing(shared: &AudioShared, info: &cpal::OutputCallbackInfo) {
    let ts = info.timestamp();
    if let Some(d) = ts.playback.duration_since(&ts.callback) {
        let sr = shared.recording.sample_rate.load(Ordering::Relaxed);
        let samples = (d.as_secs_f64() * f64::from(sr)).round() as u32;
        shared
            .recording
            .output_latency
            .store(samples, Ordering::Relaxed);
    }
}

/// Engine input of the RT renderer: hardware input from the cpal input ring, the loopback
/// of the engine's own output, or silence. Owned by `RtRenderer` (audio thread).
pub struct InputFeed {
    shared: Arc<AudioShared>,
    clock: Option<Arc<AtomicU64>>,
    feed: Feed,
    l: Vec<f32>,
    r: Vec<f32>,
    max_block: usize,
}

enum Feed {
    Silent,
    Ring(Consumer<f32>),
    Loopback {
        hist: [Vec<f32>; 2],
        /// Frames written so far (mod the history length).
        pos: usize,
        delay: usize,
    },
}

impl InputFeed {
    /// Non-RT (allocates the ring/history). The ring producer is parked in `shared` for the
    /// input stream.
    pub fn new(shared: &Arc<AudioShared>, max_block: usize, sample_rate: f64) -> Self {
        let rec = &shared.recording;
        rec.sample_rate
            .store(sample_rate.round() as u32, Ordering::Relaxed);
        rec.block.store(max_block as u32, Ordering::Relaxed);
        let mut inner = rec.inner();
        let feed = match inner.mode {
            FeedMode::Silent => Feed::Silent,
            FeedMode::Device => {
                let (tx, rx) = RingBuffer::new(max_block * INPUT_RING_BLOCKS * INPUT_CHANNELS);
                inner.pending_input = Some(tx);
                Feed::Ring(rx)
            }
            FeedMode::Loopback { delay } => {
                let delay = delay.max(max_block);
                rec.input_latency.store(delay as u32, Ordering::Relaxed);
                let len = delay + max_block;
                Feed::Loopback {
                    hist: [vec![0.0; len], vec![0.0; len]],
                    pos: 0,
                    delay,
                }
            }
        };
        Self {
            clock: inner.engine_clock.clone(),
            shared: shared.clone(),
            feed,
            l: vec![0.0; max_block],
            r: vec![0.0; max_block],
            max_block,
        }
    }

    /// **RT.** Planar input for the next `n` (`<= max_block`) frames.
    pub fn read(&mut self, n: usize) -> [&[f32]; 2] {
        let n = n.min(self.max_block);
        if let Some(clock) = &self.clock {
            self.shared
                .recording
                .publish_clock(clock.load(Ordering::Acquire));
        }
        match &mut self.feed {
            Feed::Silent => {
                self.l[..n].fill(0.0);
                self.r[..n].fill(0.0);
            }
            Feed::Ring(rx) => {
                let mut avail = rx.slots() / INPUT_CHANNELS;
                // Clock drift between input and output devices: never let the backlog
                // (and so the latency) grow without bound.
                let limit = n + self.max_block * 4;
                if avail > limit {
                    let skip = avail - (n + self.max_block * 2);
                    if let Ok(c) = rx.read_chunk(skip * INPUT_CHANNELS) {
                        c.commit_all();
                        avail -= skip;
                    }
                }
                let rec = &self.shared.recording;
                let prev = f32::from_bits(rec.backlog.load(Ordering::Relaxed));
                let next = prev + (avail as f32 - prev) * 0.05;
                rec.backlog.store(next.to_bits(), Ordering::Relaxed);
                let take = avail.min(n);
                match rx.read_chunk(take * INPUT_CHANNELS) {
                    Ok(chunk) => {
                        let (a, b) = chunk.as_slices();
                        for (i, s) in a.iter().chain(b).enumerate() {
                            let (f, ch) = (i / INPUT_CHANNELS, i % INPUT_CHANNELS);
                            if ch == 0 {
                                self.l[f] = *s;
                            } else {
                                self.r[f] = *s;
                            }
                        }
                        chunk.commit_all();
                    }
                    Err(_) => {
                        self.l[..take].fill(0.0);
                        self.r[..take].fill(0.0);
                    }
                }
                self.l[take..n].fill(0.0);
                self.r[take..n].fill(0.0);
            }
            Feed::Loopback { hist, pos, delay } => {
                let len = hist[0].len();
                // `delay >= n`: every sample read was written by an earlier block.
                let start = (*pos + len - *delay) % len;
                for i in 0..n {
                    let j = (start + i) % len;
                    self.l[i] = hist[0][j];
                    self.r[i] = hist[1][j];
                }
            }
        }
        [&self.l[..n], &self.r[..n]]
    }

    /// **RT.** The engine rendered `l`/`r` for the frames just read (loopback input).
    pub fn after_process(&mut self, l: &[f32], r: &[f32]) {
        if let Feed::Loopback { hist, pos, .. } = &mut self.feed {
            let len = hist[0].len();
            for (i, (sl, sr)) in l.iter().zip(r).enumerate() {
                let j = (*pos + i) % len;
                hist[0][j] = *sl;
                hist[1][j] = *sr;
            }
            *pos = (*pos + l.len()) % len;
        }
    }
}

/// Take the engine's recording rings (bridge construction): start the writer thread and
/// share the engine clock and live-MIDI producer.
pub fn attach(handle: &mut EngineHandle, audio: &Arc<AudioShared>) {
    let Some(RecordingIo {
        capture,
        midi_in,
        midi_out,
        clock,
    }) = handle.take_recording_io()
    else {
        return;
    };
    let writer = writer::spawn(capture, midi_out);
    let mut inner = audio.recording.inner();
    inner.engine_clock = Some(clock);
    inner.midi_in = Some(Arc::new(Mutex::new(midi_in)));
    inner.writer = writer;
}

/// `RecordingCommand::ListInputs`: the input device's channels and the MIDI ports (which
/// get connected for monitoring and recording).
pub fn list_inputs(audio: &Arc<AudioShared>) -> Result<InputList, BridgeError> {
    let weak = Arc::downgrade(audio);
    let mut inner = audio.recording.inner();
    let (mode, name, channels) = (
        inner.mode,
        inner.input_device.clone().unwrap_or_default(),
        inner.input_channels,
    );
    let audio_channels = match mode {
        FeedMode::Silent => Vec::new(),
        FeedMode::Loopback { .. } => vec![
            AudioInputChannel {
                index: 0,
                name: "Loopback L".into(),
            },
            AudioInputChannel {
                index: 1,
                name: "Loopback R".into(),
            },
        ],
        FeedMode::Device => (0..channels.clamp(1, INPUT_CHANNELS as u16))
            .map(|i| AudioInputChannel {
                index: i,
                name: format!("{name} {}", i + 1),
            })
            .collect(),
    };
    let midi = inner.midi.refresh(move |data| {
        if let Some(a) = weak.upgrade() {
            a.recording.push_live(data);
        }
    });
    Ok(InputList {
        audio: audio_channels,
        midi,
    })
}

/// Send a live MIDI message to the engine as if it came from a MIDI port (tests, virtual
/// keyboards). Returns `false` if the engine is not attached or the queue is full.
pub fn inject_midi(audio: &AudioShared, data: [u8; 3]) -> bool {
    audio.recording.push_live(data)
}

/// Start capturing a session (controller thread, before engine recording is enabled).
pub fn start(audio: &AudioShared, session: &RecordSession) -> Result<(), BridgeError> {
    let rec = &audio.recording;
    let inner = rec.inner();
    let writer = inner
        .writer
        .as_ref()
        .ok_or_else(|| BridgeError::Unavailable("the engine has no recording rings".into()))?;
    let root = inner
        .projects_root
        .clone()
        .ok_or_else(|| BridgeError::Other("no project folder to record into".into()))?;
    let media_dir = root
        .join(session.project.to_string())
        .join(ether_core::protocol::model::file::MEDIA_DIR);
    if !session.audio.is_empty() {
        std::fs::create_dir_all(&media_dir).map_err(|e| BridgeError::Other(e.to_string()))?;
    }
    writer.start(writer::StartConfig {
        session: session.clone(),
        media_dir,
        latency: u64::from(rec.round_trip_latency()),
        midi_latency: u64::from(rec.midi_latency()),
        sample_rate: rec.sample_rate.load(Ordering::Relaxed),
    })
}

/// Finish the session (controller thread, after engine recording was disabled): waits
/// until the audio thread stopped capturing, then lets the writer close the takes.
pub fn stop(audio: &AudioShared, handle: &EngineHandle) -> Result<RecordedTakes, BridgeError> {
    let deadline = Instant::now() + STOP_WAIT;
    while handle.playhead().recording && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let inner = audio.recording.inner();
    let writer = inner
        .writer
        .as_ref()
        .ok_or_else(|| BridgeError::Unavailable("the engine has no recording rings".into()))?;
    writer.stop()
}

/// The ring producer and device of a pending cpal input (`FeedMode::Device` only).
#[allow(clippy::type_complexity)]
fn take_pending_input(
    shared: &AudioShared,
) -> Option<(Producer<f32>, Option<String>, Option<String>)> {
    let mut inner = shared.recording.inner();
    if inner.mode != FeedMode::Device {
        return None;
    }
    let p = inner.pending_input.take()?;
    Some((p, inner.input_host.clone(), inner.input_device.clone()))
}

fn set_input_channels(shared: &AudioShared, channels: u16) {
    shared.recording.inner().input_channels = channels;
}
