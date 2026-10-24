//! Native audio/MIDI input capture.
//!
//! ```text
//!  cpal input callback ──ring (interleaved f32, all channels)──▶ InputFeed (output callback) ──▶ Engine::process(inputs)
//!  midir callbacks ──LiveMidi (engine-sample stamped, port-tagged)──▶ engine live MIDI ring ──▶ monitored MIDI tracks (port/channel filter)
//!  Engine (recording hook) ──CaptureBlock + samples, RecordedMidi──▶ writer thread ──▶ media/rec-*.wav
//! ```
//!
//! - **Input device**: `AudioSettings::input_device` (host-handled `Engine::SetAudioConfig`
//!   `input_device`). `None` opens no input (no microphone prompt). A device name opens that
//!   cpal input device at the engine sample rate, next to the output stream, with all its
//!   input channels (up to [`MAX_INPUT_CHANNELS`]): each track picks a mono channel or a
//!   stereo pair of them (`TrackInput::Audio { first, count }`). The special
//!   name `loopback` / `loopback:<samples>` feeds the engine's own output back as input,
//!   delayed by `<samples>` (at least one engine block): a device-less round trip for tests
//!   and diagnostics, usable with the `null`/`offline` backends.
//! - **RT side**: the input callback only writes the ring; the output callback ([`InputFeed`],
//!   owned by `RtRenderer`) only reads it and publishes the (wall clock, engine sample) pair
//!   used to stamp MIDI. No locks, no allocation.
//! - **Writer thread** (one per engine, non-RT): drains the engine's capture ring, splits
//!   it into takes and streams them as 32-bit float WAV files into
//!   `<project folder>/media/` (`project_dirs`). MIDI played while recording is collected there too.
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
mod live;
mod midi;
mod writer;

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ether_controller::{BridgeError, RecordSession, RecordedTakes};
use ether_core::EngineHandle;
use ether_core::protocol::recording::{AudioInputChannel, InputList};
use ether_core::recording::rtrb::{Consumer, Producer, RingBuffer};
use ether_core::recording::{LiveMidi, RecordingIo};

use crate::audio::{AudioBackendKind, AudioSettings};
use crate::rt::AudioShared;

pub use cpal_input::list_input_devices;
pub(crate) use cpal_input::{AsioDuplex, open_input};

/// Most hardware input channels fed to the engine (`EngineConfig::input_channels` of the
/// native engine): a device with more only exposes its first `MAX_INPUT_CHANNELS`.
pub const MAX_INPUT_CHANNELS: usize = 32;
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
    /// Interleaved channels per frame in the input ring (set by the input stream before it
    /// writes its first frame; `0` = no stream).
    ring_channels: AtomicU32,
    /// Set once by [`attach`]; read lock-free by the audio thread (the stream may start
    /// before the engine rings are attached).
    engine_clock: OnceLock<Arc<AtomicU64>>,
    /// Live MIDI towards the engine: its own lock, only held for one push.
    midi_in: OnceLock<Mutex<Producer<LiveMidi>>>,
    writer: OnceLock<writer::WriterHandle>,
    /// Connected MIDI ports. Never held while taking `inner` or `midi_in`.
    midi: Mutex<midi::MidiInputs>,
    /// Port-tagged input for the controller (MIDI learn / mappings; `midi-learn`).
    control: midi::ControlQueue,
    /// Live recording view (`live-record`): writer thread → controller thread.
    live: live::LiveShared,
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
    /// Why the configured input device could not be opened (or stopped working).
    input_error: Option<String>,
    /// Producer of the current feed's ring, taken by the input stream.
    pending_input: Option<Producer<f32>>,
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
            ring_channels: AtomicU32::new(0),
            engine_clock: OnceLock::new(),
            midi_in: OnceLock::new(),
            writer: OnceLock::new(),
            midi: Mutex::new(midi::MidiInputs::default()),
            control: midi::ControlQueue::default(),
            live: live::LiveShared::default(),
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

    /// Root of the project store: takes are written to `<project folder>/media/` (`project_dirs`).
    pub fn set_projects_root(&self, root: PathBuf) {
        self.inner().projects_root = Some(root);
    }

    /// The project store root, if set (audio streaming opens streamed media under it).
    pub fn projects_root(&self) -> Option<PathBuf> {
        self.inner().projects_root.clone()
    }

    /// Why the configured audio input is unavailable (device missing, unsupported rate,
    /// permission denied), if it is.
    pub fn input_error(&self) -> Option<String> {
        self.inner().input_error.clone()
    }

    fn set_input_error(&self, error: String) {
        self.inner().input_error = Some(error);
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

    /// Queue a live MIDI message from port `port` ([`ether_core::recording::port_key`])
    /// for the engine (MIDI thread; non-RT).
    fn push_live(&self, port: u64, data: [u8; 3]) -> bool {
        let stamp = self.stamp_now();
        let Some(midi_in) = self.midi_in.get() else {
            return false;
        };
        let mut p = midi_in.lock().unwrap_or_else(|p| p.into_inner());
        p.push(LiveMidi {
            sample_time: stamp,
            data,
            port,
        })
        .is_ok()
    }

    /// MIDI latency compensated on recorded notes: scheduling delay + output latency.
    fn midi_latency(&self) -> u32 {
        self.block.load(Ordering::Relaxed) + self.output_latency.load(Ordering::Relaxed)
    }
}

pub(crate) fn parse_loopback(name: &str) -> Option<usize> {
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
            // Tests: a fake device feeds the ring itself (`take_pending_input`).
            #[cfg(test)]
            None if name == tests::FAKE_DEVICE => FeedMode::Device,
            None if settings.backend == AudioBackendKind::Cpal => FeedMode::Device,
            None => FeedMode::Silent,
        },
    };
    let mut inner = rec.inner();
    inner.mode = mode;
    inner.input_device = settings.input_device.clone();
    inner.input_host = settings.host.clone();
    inner.input_channels = 0;
    inner.input_error = None;
    inner.pending_input = None;
    rec.input_latency.store(0, Ordering::Relaxed);
    rec.output_latency.store(0, Ordering::Relaxed);
    rec.backlog.store(0f32.to_bits(), Ordering::Relaxed);
    rec.ring_channels.store(0, Ordering::Relaxed);
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
    feed: Feed,
    /// Planar input, [`MAX_INPUT_CHANNELS`] × max block.
    bufs: Vec<Vec<f32>>,
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
                let (tx, rx) = RingBuffer::new(max_block * INPUT_RING_BLOCKS * MAX_INPUT_CHANNELS);
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
            shared: shared.clone(),
            feed,
            bufs: (0..MAX_INPUT_CHANNELS)
                .map(|_| vec![0.0; max_block])
                .collect(),
            max_block,
        }
    }

    /// **RT.** Planar input for the next `n` (`<= max_block`) frames: one slice per hardware
    /// channel (empty slices past the input's channels).
    pub fn read(&mut self, n: usize) -> [&[f32]; MAX_INPUT_CHANNELS] {
        let n = n.min(self.max_block);
        let rec = &self.shared.recording;
        if let Some(clock) = rec.engine_clock.get() {
            rec.publish_clock(clock.load(Ordering::Acquire));
        }
        let channels = match &mut self.feed {
            Feed::Silent => {
                self.bufs[0][..n].fill(0.0);
                self.bufs[1][..n].fill(0.0);
                2
            }
            Feed::Ring(rx) => {
                let ch = (rec.ring_channels.load(Ordering::Acquire) as usize)
                    .clamp(1, MAX_INPUT_CHANNELS);
                let mut avail = rx.slots() / ch;
                // Clock drift between input and output devices: never let the backlog
                // (and so the latency) grow without bound.
                let limit = n + self.max_block * 4;
                if avail > limit {
                    let skip = avail - (n + self.max_block * 2);
                    if let Ok(c) = rx.read_chunk(skip * ch) {
                        c.commit_all();
                        avail -= skip;
                    }
                }
                let prev = f32::from_bits(rec.backlog.load(Ordering::Relaxed));
                let next = prev + (avail as f32 - prev) * 0.05;
                rec.backlog.store(next.to_bits(), Ordering::Relaxed);
                let take = avail.min(n);
                match rx.read_chunk(take * ch) {
                    Ok(chunk) => {
                        let (a, b) = chunk.as_slices();
                        for (i, s) in a.iter().chain(b).enumerate() {
                            self.bufs[i % ch][i / ch] = *s;
                        }
                        chunk.commit_all();
                    }
                    Err(_) => {
                        for buf in &mut self.bufs[..ch] {
                            buf[..take].fill(0.0);
                        }
                    }
                }
                for buf in &mut self.bufs[..ch] {
                    buf[take..n].fill(0.0);
                }
                ch
            }
            Feed::Loopback { hist, pos, delay } => {
                let len = hist[0].len();
                // `delay >= n`: every sample read was written by an earlier block.
                let start = (*pos + len - *delay) % len;
                for i in 0..n {
                    let j = (start + i) % len;
                    self.bufs[0][i] = hist[0][j];
                    self.bufs[1][i] = hist[1][j];
                }
                2
            }
        };
        let mut it = self.bufs.iter();
        std::array::from_fn(|c| match it.next() {
            Some(b) if c < channels => &b[..n],
            _ => &[],
        })
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
    let rec = &audio.recording;
    if rec.engine_clock.set(clock).is_err() {
        tracing::warn!("recording already attached to another engine");
        return;
    }
    let _ = rec.midi_in.set(Mutex::new(midi_in));
    if let Some(writer) = writer::spawn(capture, midi_out, rec.live.clone()) {
        let _ = rec.writer.set(writer);
    }
}

/// `RecordingCommand::ListInputs`: the input device's channels and the MIDI ports (which
/// get connected for monitoring and recording).
pub fn list_inputs(audio: &Arc<AudioShared>) -> Result<InputList, BridgeError> {
    let weak = Arc::downgrade(audio);
    let (mode, channels) = {
        let inner = audio.recording.inner();
        (inner.mode, inner.input_channels)
    };
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
        // Numbered like the interface's inputs ("1", "2", …); the UI pairs them ("1/2").
        FeedMode::Device => (0..channels.clamp(1, MAX_INPUT_CHANNELS as u16))
            .map(|i| AudioInputChannel {
                index: i,
                name: (i + 1).to_string(),
            })
            .collect(),
    };
    let (midi, gone) = audio
        .recording
        .midi
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .refresh(&audio.recording.control, move |port, data| {
            if let Some(a) = weak.upgrade() {
                a.recording.push_live(port, data);
            }
        });
    // Closing a connection may join midir's callback thread: never under a lock.
    drop(gone);
    Ok(InputList {
        audio: audio_channels,
        midi,
    })
}

/// Send a live MIDI message to the engine from the computer MIDI keyboard
/// (`Recording::LiveMidi`; its port is `COMPUTER_KEYBOARD_PORT`). Returns `false` if the
/// engine is not attached or the queue is full.
pub fn inject_midi(audio: &AudioShared, data: [u8; 3]) -> bool {
    audio
        .recording
        .push_live(ether_core::recording::COMPUTER_KEYBOARD_KEY, data)
}

/// Like [`inject_midi`], tagged with a port id: the message also reaches the controller's
/// MIDI mappings / learn ([`drain_midi_input`]), as a hardware port's would.
pub fn inject_midi_from(audio: &AudioShared, port: &str, data: [u8; 3]) -> bool {
    audio.recording.control.inject(port, data);
    audio
        .recording
        .push_live(ether_core::recording::port_key(port), data)
}

/// Messages received from every MIDI port since the last call (`EngineBridge::poll_midi_input`).
pub fn drain_midi_input(
    audio: &AudioShared,
    out: &mut Vec<ether_core::protocol::midi_map::MidiInputEvent>,
) {
    audio.recording.control.drain(out);
}

/// Live peaks and notes captured since the last call (`EngineBridge::poll_recording`;
/// controller thread, never blocks on the audio thread).
pub fn poll_live(
    audio: &AudioShared,
    chunks: &mut Vec<ether_core::protocol::recording::LiveAudioChunk>,
    notes: &mut Vec<ether_core::protocol::recording::LiveMidiNote>,
) {
    audio.recording.live.lock().drain(chunks, notes);
}

/// Start capturing a session (controller thread, before engine recording is enabled).
pub fn start(audio: &AudioShared, session: &RecordSession) -> Result<(), BridgeError> {
    let rec = &audio.recording;
    let writer = rec
        .writer
        .get()
        .ok_or_else(|| BridgeError::Unavailable("the engine has no recording rings".into()))?;
    // Copy what's needed: the guard is never held across the (blocking) writer calls.
    let (input_error, root) = {
        let inner = rec.inner();
        (inner.input_error.clone(), inner.projects_root.clone())
    };
    if !session.audio.is_empty()
        && let Some(e) = input_error
    {
        return Err(BridgeError::Other(e));
    }
    let root = root.ok_or_else(|| BridgeError::Other("no project folder to record into".into()))?;
    let media_dir = crate::project_dirs::dir(&root, session.project)
        .join(ether_core::protocol::model::file::MEDIA_DIR);
    if !session.audio.is_empty() || !session.taps.is_empty() {
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
    let writer = audio
        .recording
        .writer
        .get()
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

/// The input stream opened with `channels` channels; it writes
/// `min(channels, MAX_INPUT_CHANNELS)` per frame into the ring.
fn set_input_channels(shared: &AudioShared, channels: u16) {
    shared.recording.inner().input_channels = channels;
    shared.recording.ring_channels.store(
        u32::from(channels).clamp(1, MAX_INPUT_CHANNELS as u32),
        Ordering::Release,
    );
}
