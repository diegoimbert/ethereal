//! Web streaming of long media (v0.3, owned by the `audio-streaming` node; CONTRACTS.md
//! §13.1).
//!
//! The engine Worker reads the media file from OPFS by byte ranges ([`RangeFs`]: the sync
//! fs host's `readRange`), decodes and resamples it chunk by chunk
//! (`ether_media::stream::ChunkDecoder`) and ships the chunks to the worklet over the
//! control ring ([`crate::proto::Frame::StreamChunk`], one frame per channel). The worklet
//! keeps them in an `ether_media::stream::StreamCache` behind the media's `AudioSource`
//! ([`WorkletStreams`]; every slot is allocated when the stream opens, a heavy frame, so
//! applying chunks never allocates). The Worker is the cache's only writer, so it mirrors
//! the slots in its `FillPolicy`; the worklet reports the play cursors and the last miss of
//! every stream ([`REPORT_STREAM`], with each state report), and the Worker keeps
//! [`READ_AHEAD_CHUNKS`] chunks ahead of them ([`WebStreams::pump`], every controller
//! tick). Before a locate or play the Worker decodes and sends the target chunks ahead of
//! the transport message (the ring is FIFO), as the native bridge primes its disk thread.
//! `WebBridge::stream_media` (shared touch in `bridge.rs`/`proto.rs`/`worklet.rs`) wires it.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use ether_controller::media_stream::StreamSource;
use ether_core::RenderGraphDesc;
use ether_core::protocol::model::{MediaId, Ulid};
use ether_media::MediaError;
use ether_media::stream::{
    CHUNK_FRAMES, ChunkDecoder, CursorState, FillPolicy, MAX_CURSORS, MediaSource, StreamCache,
    anchors, slot_count,
};

use crate::proto::{EngineMsg, encode_stream_chunk};

/// Chunks kept ahead of the play position per streamed media (web: smaller than native,
/// the SAB ring carries them).
pub const READ_AHEAD_CHUNKS: usize = 8;
/// Bytes read from OPFS per request.
pub const READ_BLOCK: usize = 256 * 1024;
/// Chunks decoded per stream per controller tick at most (about 6 s of audio per second at
/// 60 Hz and 48 kHz: plenty, and bounded so a tick stays short).
pub const CHUNKS_PER_TICK: usize = 2;
/// Don't queue more stream data while this much control traffic is still pending.
pub const MAX_PENDING_BYTES: usize = 512 * 1024;
/// Tag of a stream report (Worklet → Worker).
pub const REPORT_STREAM: u8 = b'T';

/// Milliseconds for the fill policy's timers (wall clock in the Worker).
pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }
}

/// Ranged reads of files of the Worker's file system (OPFS on the web).
pub trait RangeFs {
    /// File size in bytes.
    fn size(&mut self, path: &str) -> Result<u64, String>;
    /// Up to `len` bytes from `offset` (fewer at the end of the file).
    fn read_range(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>, String>;
}

/// Opens the byte source of a streamed media (Worker side).
pub type Opener = Box<dyn FnMut(&StreamSource) -> Result<Box<dyn MediaSource>, String>>;

/// OPFS path of a streamed media: the external path if the host resolved one, else the
/// project copy under `projects/<id>/`.
pub fn media_path(source: &StreamSource) -> String {
    match &source.external_path {
        Some(p) => p.clone(),
        None => format!(
            "{}/{}/{}",
            crate::store::PROJECTS_ROOT,
            source.project,
            source.media.file
        ),
    }
}

fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rfind('.').filter(|i| *i > 0).map(|i| &name[i + 1..])
}

/// A `Read + Seek` file over a [`RangeFs`], reading [`READ_BLOCK`]s.
pub struct RangeFile<F: RangeFs> {
    fs: F,
    path: String,
    len: u64,
    pos: u64,
    block: Vec<u8>,
    block_at: u64,
}

impl<F: RangeFs> RangeFile<F> {
    pub fn open(mut fs: F, path: String) -> Result<Self, String> {
        let len = fs.size(&path)?;
        Ok(Self {
            fs,
            path,
            len,
            pos: 0,
            block: Vec::new(),
            block_at: 0,
        })
    }
}

impl<F: RangeFs> Read for RangeFile<F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let in_block =
            self.pos >= self.block_at && self.pos < self.block_at + self.block.len() as u64;
        if !in_block {
            let n = READ_BLOCK.min((self.len - self.pos) as usize);
            self.block = self
                .fs
                .read_range(&self.path, self.pos, n)
                .map_err(std::io::Error::other)?;
            self.block_at = self.pos;
            if self.block.is_empty() {
                return Ok(0);
            }
        }
        let at = (self.pos - self.block_at) as usize;
        let n = buf.len().min(self.block.len() - at);
        buf[..n].copy_from_slice(&self.block[at..at + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<F: RangeFs> Seek for RangeFile<F> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if to < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = to as u64;
        Ok(self.pos)
    }
}

/// Wrapper asserting `Send + Sync` for byte sources that live on one thread (the web
/// Worker has exactly one; the JS fs host is not `Send`).
struct OneThread<T>(T);
// SAFETY: wasm32 Workers are single-threaded; the source never leaves the Worker thread
// (it is created, used and dropped by the controller, which runs on that thread only).
unsafe impl<T> Send for OneThread<T> {}
// SAFETY: as above, never shared across threads.
unsafe impl<T> Sync for OneThread<T> {}

impl<T: Read> Read for OneThread<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Seek> Seek for OneThread<T> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<T: RangeFs> MediaSource for OneThread<RangeFile<T>> {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.len)
    }
}

/// An [`Opener`] over a [`RangeFs`] (cloned per stream).
pub fn range_opener<F: RangeFs + Clone + 'static>(fs: F) -> Opener {
    Box::new(move |source: &StreamSource| {
        let file = RangeFile::open(fs.clone(), media_path(source))?;
        Ok(Box::new(OneThread(file)) as Box<dyn MediaSource>)
    })
}

// ─── Worker side ────────────────────────────────────────────────────────────────────────

struct WebStream {
    decoder: ChunkDecoder,
    policy: FillPolicy,
    cursors: [Option<CursorState>; MAX_CURSORS],
    frames: u64,
}

/// The Worker's streams: decoders, slot mirrors, and what the worklet reported.
#[derive(Default)]
pub struct WebStreams {
    opener: Option<Opener>,
    streams: BTreeMap<MediaId, WebStream>,
    /// Last published graph while streams exist (anchors, locate targets).
    graph: Option<RenderGraphDesc>,
    buf: Vec<Vec<f32>>,
    /// Decode errors (the chunk is sent as silence).
    pub errors: u64,
}

impl WebStreams {
    pub fn set_opener(&mut self, opener: Opener) {
        self.opener = Some(opener);
    }

    pub fn can_stream(&self) -> bool {
        self.opener.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    pub fn contains(&self, media: MediaId) -> bool {
        self.streams.contains_key(&media)
    }

    /// Open `source` at `engine_rate`: sends the open message, then the head chunks.
    pub fn open(
        &mut self,
        source: &StreamSource,
        engine_rate: u32,
        now_ms: f64,
        send: &mut dyn FnMut(Vec<Vec<u8>>),
    ) -> Result<(), MediaError> {
        let opener = self
            .opener
            .as_mut()
            .ok_or_else(|| MediaError::Unsupported("no stream file system".into()))?;
        let file = opener(source).map_err(MediaError::Decode)?;
        let ext = extension(&source.media.file);
        let decoder = ChunkDecoder::new(file, ext, source.media.frames, engine_rate)?;
        let slots = slot_count(READ_AHEAD_CHUNKS);
        let policy = FillPolicy::new(decoder.chunk_count(), slots, READ_AHEAD_CHUNKS);
        let frames = decoder.frames();
        let id = source.media.id;
        send(
            EngineMsg::StreamOpen {
                media: id,
                channels: decoder.channels(),
                frames,
                slots: slots as u32,
            }
            .encode(),
        );
        self.streams.insert(
            id,
            WebStream {
                decoder,
                policy,
                cursors: [None; MAX_CURSORS],
                frames,
            },
        );
        self.update_anchors();
        // Head and anchors right away.
        self.fill(id, usize::MAX, now_ms, send);
        Ok(())
    }

    pub fn remove(&mut self, media: MediaId) -> bool {
        self.streams.remove(&media).is_some()
    }

    /// A graph was published.
    pub fn observe_graph(&mut self, graph: &RenderGraphDesc) {
        if self.streams.is_empty() {
            self.graph = None;
            return;
        }
        self.graph = Some(graph.clone());
        self.update_anchors();
    }

    /// A `SetLoop` override.
    pub fn observe_loop(&mut self, enabled: bool, start: f64, end: f64) {
        if let Some(g) = self.graph.as_mut() {
            g.loop_enabled = enabled;
            g.loop_start = start;
            g.loop_end = end;
            self.update_anchors();
        }
    }

    fn update_anchors(&mut self) {
        let Some(graph) = &self.graph else { return };
        let Some(rate) = self
            .streams
            .values()
            .next()
            .map(|s| s.decoder.engine_rate())
        else {
            return;
        };
        let lens: BTreeMap<MediaId, u64> =
            self.streams.iter().map(|(m, s)| (*m, s.frames)).collect();
        let frames = |m: MediaId| lens.get(&m).copied();
        let mut a = anchors::anchors(graph, rate, &frames);
        for (id, s) in &mut self.streams {
            s.policy.set_anchors(a.remove(id).unwrap_or_default());
        }
    }

    /// Before a jump to `beat`: decode what plays there and send it now (ahead of the
    /// transport message).
    pub fn prime_at(&mut self, beat: f64, now_ms: f64, send: &mut dyn FnMut(Vec<Vec<u8>>)) {
        let Some(graph) = &self.graph else { return };
        let Some(rate) = self
            .streams
            .values()
            .next()
            .map(|s| s.decoder.engine_rate())
        else {
            return;
        };
        let lens: BTreeMap<MediaId, u64> =
            self.streams.iter().map(|(m, s)| (*m, s.frames)).collect();
        let frames = |m: MediaId| lens.get(&m).copied();
        let targets = anchors::positions_at(graph, beat, rate, &frames);
        let mut touched = Vec::new();
        for (id, frame) in targets {
            if let Some(s) = self.streams.get_mut(&id) {
                s.policy.urge_frame(frame, now_ms);
                touched.push(id);
            }
        }
        touched.dedup();
        for id in touched {
            // Urgent chunks (and the one after each) come first in the plan.
            self.fill(id, 6, now_ms, send);
        }
    }

    /// The worklet's report for `media`: its cursors and last miss.
    pub fn report(
        &mut self,
        media: MediaId,
        cursors: [Option<CursorState>; MAX_CURSORS],
        missed: Option<u64>,
        now_ms: f64,
    ) {
        if let Some(s) = self.streams.get_mut(&media) {
            s.cursors = cursors;
            if let Some(c) = missed {
                s.policy.urge(c, now_ms);
            }
        }
    }

    /// Periodic: keep every stream's read-ahead filled (bounded per call).
    pub fn pump(&mut self, now_ms: f64, send: &mut dyn FnMut(Vec<Vec<u8>>)) {
        let ids: Vec<MediaId> = self.streams.keys().copied().collect();
        for id in ids {
            self.fill(id, CHUNKS_PER_TICK, now_ms, send);
        }
    }

    fn fill(&mut self, id: MediaId, max: usize, now_ms: f64, send: &mut dyn FnMut(Vec<Vec<u8>>)) {
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        for _ in 0..max {
            let Some((slot, chunk)) = s.policy.next(&s.cursors, now_ms) else {
                return;
            };
            if s.decoder.decode_chunk(chunk, &mut self.buf).is_err() {
                self.errors += 1;
                for b in &mut self.buf {
                    b.clear();
                    b.resize(CHUNK_FRAMES, 0.0);
                }
            }
            s.policy.wrote(slot, Some(chunk));
            send(encode_stream_chunk(id, slot as u32, chunk, &self.buf));
        }
    }
}

// ─── Worklet side ───────────────────────────────────────────────────────────────────────

// [T][count u16] then per stream: [media u128][missed u64 (chunk + 1, 0 = none)]
//   [MAX_CURSORS x (frame + 1 u64 (0 = none), stamp u64, backwards u8)]
const CURSOR_BYTES: usize = 8 + 8 + 1;
const STREAM_REPORT_BYTES: usize = 16 + 8 + MAX_CURSORS * CURSOR_BYTES;

/// The worklet's streamed media caches.
#[derive(Default)]
pub struct WorkletStreams {
    caches: BTreeMap<MediaId, Arc<StreamCache>>,
    report: Vec<u8>,
}

impl WorkletStreams {
    /// Allocate the cache of a new stream (heavy: the control loop stops after it).
    pub fn open(
        &mut self,
        media: MediaId,
        channels: u16,
        frames: u64,
        slots: u32,
    ) -> Arc<StreamCache> {
        let cache = Arc::new(StreamCache::new(channels, frames, slots as usize));
        self.caches.insert(media, cache.clone());
        self.report = Vec::with_capacity(3 + self.caches.len() * STREAM_REPORT_BYTES);
        cache
    }

    pub fn remove(&mut self, media: MediaId) {
        self.caches.remove(&media);
    }

    pub fn is_empty(&self) -> bool {
        self.caches.is_empty()
    }

    /// Apply one channel of a chunk (no allocation).
    pub fn chunk(
        &self,
        media: MediaId,
        slot: u32,
        chunk: u64,
        channel: u16,
        flags: u8,
        samples: &[u8],
    ) {
        let Some(cache) = self.caches.get(&media) else {
            // Unloaded meanwhile.
            return;
        };
        let slot = slot as usize;
        if flags & crate::proto::STREAM_BEGIN != 0 {
            cache.begin_slot(slot, chunk);
        }
        cache.write_channel_bytes(slot, channel, 0, samples);
        if flags & crate::proto::STREAM_END != 0 {
            cache.end_slot(slot);
        }
    }

    /// **RT** (no allocation): encode the stream report into the reused buffer.
    pub fn encode_report(&mut self) -> &[u8] {
        let out = &mut self.report;
        out.clear();
        out.push(REPORT_STREAM);
        out.extend_from_slice(&(self.caches.len().min(u16::MAX as usize) as u16).to_le_bytes());
        for (id, cache) in &self.caches {
            out.extend_from_slice(&id.0.0.to_le_bytes());
            out.extend_from_slice(&cache.take_missed().map_or(0, |c| c + 1).to_le_bytes());
            for c in cache.cursors() {
                let (pos, stamp, back) =
                    c.map_or((0, 0, 0), |c| (c.frame + 1, c.stamp, c.backwards as u8));
                out.extend_from_slice(&pos.to_le_bytes());
                out.extend_from_slice(&stamp.to_le_bytes());
                out.push(back);
            }
        }
        &self.report
    }
}

/// One stream's entry of a [`REPORT_STREAM`].
pub struct StreamReport {
    pub media: MediaId,
    pub missed: Option<u64>,
    pub cursors: [Option<CursorState>; MAX_CURSORS],
}

/// Decode a [`REPORT_STREAM`] (Worker side).
pub fn decode_report(bytes: &[u8]) -> Result<Vec<StreamReport>, String> {
    let bad = || "truncated stream report".to_string();
    let mut r = bytes;
    let mut take = |n: usize| -> Result<&[u8], String> {
        if r.len() < n {
            return Err(bad());
        }
        let (a, b) = r.split_at(n);
        r = b;
        Ok(a)
    };
    if take(1)?[0] != REPORT_STREAM {
        return Err("not a stream report".into());
    }
    let n = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let media = MediaId(Ulid(u128::from_le_bytes(take(16)?.try_into().unwrap())));
        let missed = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let mut cursors = [None; MAX_CURSORS];
        for c in &mut cursors {
            let pos = u64::from_le_bytes(take(8)?.try_into().unwrap());
            let stamp = u64::from_le_bytes(take(8)?.try_into().unwrap());
            let back = take(1)?[0] != 0;
            *c = (pos != 0).then(|| CursorState {
                frame: pos - 1,
                stamp,
                backwards: back,
            });
        }
        out.push(StreamReport {
            media,
            missed: missed.checked_sub(1),
            cursors,
        });
    }
    Ok(out)
}
