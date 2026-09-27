//! The take writer thread: engine capture ring → latency-compensated takes → WAV files.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use ether_controller::{AudioTake, BridgeError, RecordSession, RecordedMidi, RecordedTakes};
use ether_core::recording::rtrb::Consumer;
use ether_core::recording::{CaptureBlock, CaptureReader, RecordedMidi as EngineMidi};

/// How often the writer drains the engine rings.
const POLL: Duration = Duration::from_millis(5);
/// How long `stop` waits for the files to be closed.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct StartConfig {
    pub session: RecordSession,
    pub media_dir: PathBuf,
    /// Round-trip latency compensated on audio (samples).
    pub latency: u64,
    /// Latency compensated on MIDI (samples).
    pub midi_latency: u64,
    pub sample_rate: u32,
}

enum Msg {
    /// Acknowledged once the session is installed (so no captured block can be missed).
    Start(Box<StartConfig>, Sender<()>),
    Stop(Sender<Result<RecordedTakes, String>>),
}

/// Control side of the writer thread (the thread exits when this is dropped).
pub(super) struct WriterHandle {
    tx: Sender<Msg>,
}

pub(super) fn spawn(capture: CaptureReader, midi: Consumer<EngineMidi>) -> Option<WriterHandle> {
    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("ether-rec-writer".into())
        .spawn(move || run(rx, capture, midi))
        .map_err(|e| tracing::error!(%e, "could not start the recording writer"))
        .ok()?;
    Some(WriterHandle { tx })
}

impl WriterHandle {
    pub fn start(&self, config: StartConfig) -> Result<(), BridgeError> {
        let (tx, rx) = bounded(1);
        self.tx
            .send(Msg::Start(Box::new(config), tx))
            .map_err(|_| BridgeError::Unavailable("recording writer stopped".into()))?;
        rx.recv_timeout(STOP_TIMEOUT)
            .map_err(|_| BridgeError::Other("recording writer timed out".into()))
    }

    pub fn stop(&self) -> Result<RecordedTakes, BridgeError> {
        let (tx, rx) = bounded(1);
        self.tx
            .send(Msg::Stop(tx))
            .map_err(|_| BridgeError::Unavailable("recording writer stopped".into()))?;
        rx.recv_timeout(STOP_TIMEOUT)
            .map_err(|_| BridgeError::Other("recording writer timed out".into()))?
            .map_err(BridgeError::Other)
    }
}

fn run(rx: Receiver<Msg>, mut capture: CaptureReader, mut midi: Consumer<EngineMidi>) {
    let mut session: Option<Session> = None;
    let mut buf = Vec::new();
    loop {
        let msg = match rx.recv_timeout(POLL) {
            // A new session owns everything captured from now on (the engine only
            // captures after `start` returned and the controller enabled recording).
            Ok(Msg::Start(config, ack)) => {
                if let Some(old) = session.take() {
                    let _ = old.finish();
                }
                session = Some(Session::new(*config, capture.channels()));
                let _ = ack.send(());
                continue;
            }
            other => other,
        };
        while let Some(block) = capture.next_block(&mut buf) {
            if let Some(s) = &mut session {
                s.block(&block, &buf);
            }
            buf.clear();
        }
        while let Ok(m) = midi.pop() {
            if let Some(s) = &mut session {
                s.midi(m);
            }
        }
        match msg {
            // Everything captured before the stop was drained above.
            Ok(Msg::Stop(reply)) => {
                let result = session
                    .take()
                    .map_or_else(|| Ok(RecordedTakes::default()), Session::finish);
                let _ = reply.send(result);
            }
            Ok(Msg::Start(..)) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// A captured sub-block (see [`CaptureBlock`]), tagged with its contiguous run.
#[derive(Clone, Copy, Debug)]
struct Segment {
    start: u64,
    frames: u64,
    position: f64,
    bps: f64,
    run: u64,
}

struct Take {
    run: u64,
    start: f64,
    files: Vec<TakeFile>,
}

struct TakeFile {
    track: ether_core::protocol::model::TrackId,
    rel: String,
    first: usize,
    count: usize,
    out: WavWriter,
}

struct Session {
    config: StartConfig,
    channels: usize,
    segments: VecDeque<Segment>,
    runs: u64,
    current: Option<Take>,
    takes: u32,
    done: Vec<AudioTake>,
    midi: Vec<RecordedMidi>,
    error: Option<String>,
}

impl Session {
    fn new(config: StartConfig, channels: usize) -> Self {
        Self {
            config,
            channels: channels.max(1),
            segments: VecDeque::new(),
            runs: 0,
            current: None,
            takes: 0,
            done: Vec::new(),
            midi: Vec::new(),
            error: None,
        }
    }

    fn keep(&self, position: f64) -> bool {
        let s = &self.config.session;
        position >= s.keep_from - 1e-9 && s.keep_until.is_none_or(|u| position < u - 1e-9)
    }

    fn block(&mut self, b: &CaptureBlock, samples: &[f32]) {
        if self.config.session.audio.is_empty() || self.error.is_some() {
            return;
        }
        let run = match self.segments.back() {
            Some(p)
                if p.start + p.frames == b.sample_time
                    && (p.position + p.frames as f64 * p.bps - b.position).abs() < 1e-6 =>
            {
                p.run
            }
            _ => {
                self.runs += 1;
                self.runs
            }
        };
        self.segments.push_back(Segment {
            start: b.sample_time,
            frames: u64::from(b.frames),
            position: b.position,
            bps: b.beats_per_sample,
            run,
        });
        let latency = self.config.latency;
        for i in 0..u64::from(b.frames) {
            let k = b.sample_time + i;
            let frame = &samples[i as usize * self.channels..(i as usize + 1) * self.channels];
            // Frame `k` was played while the performer heard engine sample `k - latency`.
            let Some(h) = k.checked_sub(latency) else {
                self.gap();
                continue;
            };
            while self
                .segments
                .front()
                .is_some_and(|s| s.start + s.frames <= h)
            {
                self.segments.pop_front();
            }
            let Some(seg) = self.segments.front().copied().filter(|s| s.start <= h) else {
                self.gap();
                continue;
            };
            let position = seg.position + (h - seg.start) as f64 * seg.bps;
            if !self.keep(position) {
                self.gap();
                continue;
            }
            if self.current.as_ref().is_none_or(|t| t.run != seg.run) {
                self.gap();
                self.open_take(seg.run, position);
            }
            if let Some(take) = &mut self.current {
                let mut failed = None;
                for f in &mut take.files {
                    for c in 0..f.count {
                        let s = frame.get(f.first + c).copied().unwrap_or(0.0);
                        if let Err(e) = f.out.sample(s) {
                            failed = Some(e.to_string());
                        }
                    }
                    f.out.frames += 1;
                }
                if let Some(e) = failed {
                    self.error = Some(e);
                    return;
                }
            }
        }
    }

    fn open_take(&mut self, run: u64, start: f64) {
        self.takes += 1;
        let cfg = &self.config;
        let mut files = Vec::new();
        for target in &cfg.session.audio {
            let name = format!(
                "rec-{}-{}-{}.wav",
                cfg.session.tag, target.track, self.takes
            );
            let count = usize::from(target.count.clamp(1, 2));
            match WavWriter::create(&cfg.media_dir.join(&name), count as u16, cfg.sample_rate) {
                Ok(out) => files.push(TakeFile {
                    track: target.track,
                    rel: format!("{}/{name}", ether_core::protocol::model::file::MEDIA_DIR),
                    first: usize::from(target.first),
                    count,
                    out,
                }),
                Err(e) => {
                    self.error = Some(format!("{}: {e}", name));
                    return;
                }
            }
        }
        self.current = Some(Take { run, start, files });
    }

    /// The current take (if any) ends here.
    fn gap(&mut self) {
        let Some(take) = self.current.take() else {
            return;
        };
        for f in take.files {
            let frames = f.out.frames;
            match f.out.finish() {
                Ok(()) => self.done.push(AudioTake {
                    track: f.track,
                    file: f.rel,
                    start: take.start,
                    frames,
                    channels: f.count as u16,
                    sample_rate: self.config.sample_rate,
                }),
                Err(e) => self.error = Some(e.to_string()),
            }
        }
    }

    fn midi(&mut self, m: EngineMidi) {
        if !self.config.session.midi {
            return;
        }
        let position = m.position - self.config.midi_latency as f64 * m.beats_per_sample;
        if self.keep(position) {
            self.midi.push(RecordedMidi {
                position,
                data: m.data,
            });
        }
    }

    fn finish(mut self) -> Result<RecordedTakes, String> {
        self.gap();
        if let Some(e) = self.error {
            return Err(e);
        }
        self.midi.sort_by(|a, b| a.position.total_cmp(&b.position));
        Ok(RecordedTakes {
            audio: self.done,
            midi: self.midi,
            latency: self.config.latency as u32,
        })
    }
}

/// Streams a 32-bit float WAV; sizes are patched in [`WavWriter::finish`].
pub(super) struct WavWriter {
    out: BufWriter<File>,
    pub frames: u64,
    channels: u16,
}

pub(super) fn wav_header(channels: u16, sample_rate: u32, frames: u64) -> [u8; 44] {
    let data = (frames * u64::from(channels) * 4).min(u64::from(u32::MAX - 36)) as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&3u16.to_le_bytes()); // IEEE float
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&(sample_rate * u32::from(channels) * 4).to_le_bytes());
    h[32..34].copy_from_slice(&(channels * 4).to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

impl WavWriter {
    fn create(path: &Path, channels: u16, sample_rate: u32) -> std::io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&wav_header(channels, sample_rate, 0))?;
        Ok(Self {
            out,
            frames: 0,
            channels,
        })
    }

    fn sample(&mut self, s: f32) -> std::io::Result<()> {
        self.out.write_all(&s.to_le_bytes())
    }

    fn finish(mut self) -> std::io::Result<()> {
        let header = wav_header(self.channels, 0, self.frames);
        self.out.flush()?;
        let file = self.out.get_mut();
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&header[4..8])?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&header[40..44])?;
        file.sync_all()
    }
}
