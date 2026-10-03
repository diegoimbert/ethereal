//! Disk streaming of long media (v0.3, owned by the `audio-streaming` node; CONTRACTS.md
//! §13.1). (`crate::stream` is the collab "listen on peer" sender, unrelated.)
//!
//! `NativeBridge::stream_media` (shared touch in `bridge.rs`) opens the media file (the
//! project copy under the store root, or the external reference), creates an
//! `ether_media::stream::StreamFiller` (decoder + lock-free cache + fill policy), primes
//! the head of the file, registers the cache as the media's `AudioSource` with
//! `EngineHandle::add_source`, and hands the filler to **one shared reader thread** here
//! ([`DiskStreams`]). The thread keeps every streamed media's cache filled around the play
//! cursors the engine hints (`AudioSource::prefetch_hint`, and every read), the anchors
//! the bridge derives from each published graph (clip starts, clip loops, the transport
//! loop) and the positions it primes before a locate or play, reading up to
//! [`READ_AHEAD_CHUNKS`] chunks ahead. It only talks to the audio thread through the
//! caches. Underruns are counted by the engine (`EngineOutputs::underruns`) and reported
//! by the controller (`MediaEvent::StreamUnderruns`).
//!
//! The thread decodes one chunk per stream per round (round robin), handling commands
//! between chunks so a prime is served next; idle, it wakes every [`IDLE_WAKE`] to look for
//! moved cursors and misses (the audio thread never signals: no syscalls there).

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use ether_controller::media_stream::StreamSource;
use ether_core::protocol::model::{MediaId, ProjectId};
use ether_media::MediaError;
use ether_media::stream::{ChunkDecoder, StreamCache, StreamFiller};

/// Chunks kept decoded ahead of the play position per streamed media.
pub const READ_AHEAD_CHUNKS: usize = 12;
/// Reader thread poll interval while there is nothing to decode.
pub const IDLE_WAKE: Duration = Duration::from_millis(3);
/// Longest a locate/play waits for its chunks (controller thread).
pub const PRIME_TIMEOUT: Duration = Duration::from_millis(150);

enum Cmd {
    Add(MediaId, Box<StreamFiller>),
    Remove(MediaId),
    Anchors(MediaId, Vec<u64>),
    /// Decode these `(media, engine frame)` chunks first; answer when they are resident.
    Prime(Vec<(MediaId, u64)>, Sender<()>),
}

/// A streamed media as the bridge sees it.
#[derive(Clone)]
pub struct StreamInfo {
    pub cache: Arc<StreamCache>,
    /// Length at the engine rate.
    pub frames: u64,
}

/// The shared reader thread and the streams it serves.
pub struct DiskStreams {
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
    streams: BTreeMap<MediaId, StreamInfo>,
    epoch: Instant,
}

impl Default for DiskStreams {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskStreams {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        let epoch = Instant::now();
        let thread = std::thread::Builder::new()
            .name("ether-disk".into())
            .spawn(move || run(rx, epoch))
            .expect("spawn disk stream thread");
        Self {
            tx,
            thread: Some(thread),
            streams: BTreeMap::new(),
            epoch,
        }
    }

    fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    /// Open `source` for streaming at `engine_rate`: the file is probed and the head of the
    /// media decoded before this returns (so a clip starting at 0 plays at once).
    pub fn open(
        &mut self,
        source: &StreamSource,
        projects_root: Option<&Path>,
        engine_rate: u32,
    ) -> Result<StreamInfo, MediaError> {
        let path = media_path(source, projects_root)?;
        let file = File::open(&path)
            .map_err(|e| MediaError::Decode(format!("{}: {e}", path.display())))?;
        let ext = path.extension().and_then(|e| e.to_str());
        let decoder = ChunkDecoder::new(Box::new(file), ext, source.media.frames, engine_rate)?;
        let mut filler = StreamFiller::new(decoder, READ_AHEAD_CHUNKS);
        filler.fill_all(self.now_ms());
        let info = StreamInfo {
            cache: filler.cache().clone(),
            frames: ether_core::AudioSource::frames(&**filler.cache()),
        };
        let id = source.media.id;
        self.streams.insert(id, info.clone());
        let _ = self.tx.send(Cmd::Add(id, Box::new(filler)));
        Ok(info)
    }

    /// Stop streaming `media` (its cache is dropped once the engine lets go of it).
    pub fn remove(&mut self, media: MediaId) -> bool {
        let had = self.streams.remove(&media).is_some();
        if had {
            let _ = self.tx.send(Cmd::Remove(media));
        }
        had
    }

    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    pub fn get(&self, media: MediaId) -> Option<&StreamInfo> {
        self.streams.get(&media)
    }

    /// Engine-rate length of a streamed media.
    pub fn frames(&self, media: MediaId) -> Option<u64> {
        self.streams.get(&media).map(|s| s.frames)
    }

    /// Replace every stream's anchors (engine frames; streams missing from `anchors` get
    /// none).
    pub fn set_anchors(&mut self, mut anchors: BTreeMap<MediaId, Vec<u64>>) {
        for id in self.streams.keys() {
            let a = anchors.remove(id).unwrap_or_default();
            let _ = self.tx.send(Cmd::Anchors(*id, a));
        }
    }

    /// Decode the chunks at these positions first and wait (at most `timeout`) until they
    /// are resident. Returns `false` on timeout (playback may underrun briefly).
    pub fn prime(&self, targets: Vec<(MediaId, u64)>, timeout: Duration) -> bool {
        let targets: Vec<_> = targets
            .into_iter()
            .filter(|(m, _)| self.streams.contains_key(m))
            .collect();
        if targets.is_empty() {
            return true;
        }
        let (done_tx, done_rx) = bounded(1);
        if self.tx.send(Cmd::Prime(targets, done_tx)).is_err() {
            return false;
        }
        done_rx.recv_timeout(timeout).is_ok()
    }
}

impl Drop for DiskStreams {
    fn drop(&mut self) {
        // Closing the channel stops the thread.
        let (tx, _) = unbounded();
        drop(std::mem::replace(&mut self.tx, tx));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The file of `source`: the external reference, else `<root>/<project>/<file>` (a
/// relative path inside the project folder).
pub fn media_path(
    source: &StreamSource,
    projects_root: Option<&Path>,
) -> Result<PathBuf, MediaError> {
    if let Some(p) = &source.external_path {
        return Ok(PathBuf::from(p));
    }
    let root = projects_root.ok_or_else(|| MediaError::Decode("no project folder".into()))?;
    project_file(root, source.project, &source.media.file)
}

fn project_file(root: &Path, project: ProjectId, file: &str) -> Result<PathBuf, MediaError> {
    let rel = Path::new(file);
    if file.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(MediaError::Decode(format!("invalid media path {file:?}")));
    }
    Ok(root.join(project.to_string()).join(rel))
}

struct Pending {
    targets: Vec<(MediaId, u64)>,
    done: Sender<()>,
}

fn run(rx: Receiver<Cmd>, epoch: Instant) {
    let mut streams: BTreeMap<MediaId, Box<StreamFiller>> = BTreeMap::new();
    let mut pending: Vec<Pending> = Vec::new();
    let now = || epoch.elapsed().as_secs_f64() * 1000.0;
    let mut idle = false;
    loop {
        // Commands: block briefly only when there was nothing to decode.
        let first = if idle {
            match rx.recv_timeout(IDLE_WAKE) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.try_recv() {
                Ok(c) => Some(c),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        };
        for cmd in first
            .into_iter()
            .chain(std::iter::from_fn(|| rx.try_recv().ok()))
        {
            match cmd {
                Cmd::Add(id, f) => {
                    streams.insert(id, f);
                }
                Cmd::Remove(id) => {
                    streams.remove(&id);
                }
                Cmd::Anchors(id, a) => {
                    if let Some(f) = streams.get_mut(&id) {
                        f.policy_mut().set_anchors(a);
                    }
                }
                Cmd::Prime(targets, done) => {
                    let t = now();
                    for (id, frame) in &targets {
                        if let Some(f) = streams.get_mut(id) {
                            f.urge_frames(&[*frame], t);
                        }
                    }
                    pending.push(Pending { targets, done });
                }
            }
        }
        // Primes first: decode until they are resident (then answer).
        let t = now();
        let mut worked = false;
        for p in &pending {
            for (id, frame) in &p.targets {
                if let Some(f) = streams.get_mut(id) {
                    while !f.primed(&[*frame]) && f.fill_one(t) {
                        worked = true;
                    }
                }
            }
        }
        pending.retain(|p| {
            let ready = p
                .targets
                .iter()
                .all(|(id, frame)| streams.get(id).is_none_or(|f| f.primed(&[*frame])));
            if ready {
                let _ = p.done.send(());
            }
            !ready
        });
        // One chunk per stream per round.
        for f in streams.values_mut() {
            worked |= f.fill_one(t);
        }
        idle = !worked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::{MediaLocation, MediaRef, Ulid};

    #[test]
    fn project_paths_stay_inside_the_project() {
        let root = Path::new("/r");
        let p = ProjectId::NIL;
        assert_eq!(
            project_file(root, p, "media/a.wav").unwrap(),
            Path::new("/r").join(p.to_string()).join("media/a.wav")
        );
        for bad in ["", "../x.wav", "/etc/passwd", "media/../../x"] {
            assert!(project_file(root, p, bad).is_err(), "{bad}");
        }
        let src = StreamSource {
            project: p,
            media: MediaRef {
                id: MediaId(Ulid(1)),
                name: "a".into(),
                file: "media/a.wav".into(),
                sample_rate: 48_000,
                channels: 2,
                frames: 1,
                hash: None,
                location: MediaLocation::Project,
            },
            external_path: Some("/abs/a.wav".into()),
            engine_sample_rate: 48_000,
        };
        assert_eq!(media_path(&src, None).unwrap(), Path::new("/abs/a.wav"));
    }
}
