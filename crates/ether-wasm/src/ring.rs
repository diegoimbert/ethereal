//! Single-producer / single-consumer byte ring over shared memory, carrying
//! length-prefixed messages.
//!
//! On the web the memory is a `SharedArrayBuffer` shared by two separate wasm instances
//! (controller Worker and AudioWorklet), so the ring lives in JS memory and each side
//! copies bytes in/out ([`RingMemory`]). Natively (tests) it is [`HeapMemory`].
//!
//! # Layout (`SharedArrayBuffer`)
//!
//! ```text
//! bytes 0..4    head: u32, total bytes ever written (wrapping), written by the producer
//! bytes 4..8    tail: u32, total bytes ever read (wrapping), written by the consumer
//! bytes 8..16   reserved
//! bytes 16..    data: `capacity` bytes (power of two)
//! ```
//!
//! # Stream format
//!
//! The ring is a byte stream of messages, each `[len: u32 LE][len bytes]`. A message may be
//! larger than the ring: the writer streams it in pieces as space frees up
//! ([`RingWriter::flush`]) and the reader reassembles it across drains, so big payloads
//! (graph snapshots, decoded media) need no separate channel and stay ordered with every
//! other message.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// Bytes before the data area in the shared buffer.
pub const HEADER_BYTES: usize = 16;
/// Largest message the reader accepts (a corrupt length must not trigger a huge allocation).
pub const MAX_MESSAGE: usize = 1 << 30;

/// Shared memory backing a ring. `write`/`read` never wrap: callers split at the end.
pub trait RingMemory {
    /// Data capacity in bytes (a power of two).
    fn capacity(&self) -> usize;
    fn load_head(&self) -> u32;
    fn store_head(&self, v: u32);
    fn load_tail(&self) -> u32;
    fn store_tail(&self, v: u32);
    /// Copy `bytes` to data offset `pos` (`pos + bytes.len() <= capacity`).
    fn write(&self, pos: usize, bytes: &[u8]);
    /// Copy data at offset `pos` into `out` (`pos + out.len() <= capacity`).
    fn read(&self, pos: usize, out: &mut [u8]);
}

/// Heap-backed shared memory for native tests (clone = same ring).
#[derive(Clone)]
pub struct HeapMemory(Arc<HeapInner>);

struct HeapInner {
    head: AtomicU32,
    tail: AtomicU32,
    data: Box<[AtomicU8]>,
}

impl HeapMemory {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two() && capacity >= 16);
        Self(Arc::new(HeapInner {
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            data: (0..capacity).map(|_| AtomicU8::new(0)).collect(),
        }))
    }
}

impl RingMemory for HeapMemory {
    fn capacity(&self) -> usize {
        self.0.data.len()
    }
    fn load_head(&self) -> u32 {
        self.0.head.load(Ordering::Acquire)
    }
    fn store_head(&self, v: u32) {
        self.0.head.store(v, Ordering::Release)
    }
    fn load_tail(&self) -> u32 {
        self.0.tail.load(Ordering::Acquire)
    }
    fn store_tail(&self, v: u32) {
        self.0.tail.store(v, Ordering::Release)
    }
    fn write(&self, pos: usize, bytes: &[u8]) {
        for (cell, b) in self.0.data[pos..pos + bytes.len()].iter().zip(bytes) {
            cell.store(*b, Ordering::Relaxed);
        }
    }
    fn read(&self, pos: usize, out: &mut [u8]) {
        for (b, cell) in out.iter_mut().zip(&self.0.data[pos..]) {
            *b = cell.load(Ordering::Relaxed);
        }
    }
}

/// Copy `bytes` into the ring at stream offset `at` (handles wrap-around).
fn put<M: RingMemory>(mem: &M, at: u32, bytes: &[u8]) {
    let cap = mem.capacity();
    let pos = at as usize & (cap - 1);
    let first = bytes.len().min(cap - pos);
    mem.write(pos, &bytes[..first]);
    if first < bytes.len() {
        mem.write(0, &bytes[first..]);
    }
}

/// Copy from the ring at stream offset `at` into `out` (handles wrap-around).
fn get<M: RingMemory>(mem: &M, at: u32, out: &mut [u8]) {
    let cap = mem.capacity();
    let pos = at as usize & (cap - 1);
    let first = out.len().min(cap - pos);
    mem.read(pos, &mut out[..first]);
    if first < out.len() {
        mem.read(0, &mut out[first..]);
    }
}

fn free_space<M: RingMemory>(mem: &M) -> usize {
    let used = mem.load_head().wrapping_sub(mem.load_tail()) as usize;
    mem.capacity().saturating_sub(used)
}

/// Producer half.
///
/// Two modes: [`RingWriter::send`] queues messages of any size in a local outbox and
/// streams them as space frees up (non-RT side: the controller Worker);
/// [`RingWriter::try_send_now`] writes a whole message immediately or drops it, without
/// allocating (RT side: the AudioWorklet's reports).
pub struct RingWriter<M: RingMemory> {
    mem: M,
    /// Framed messages (`len` prefix included) not fully written yet.
    outbox: VecDeque<Vec<u8>>,
    /// Bytes of `outbox.front()` already written.
    written: usize,
    queued: usize,
}

impl<M: RingMemory> RingWriter<M> {
    pub fn new(mem: M) -> Self {
        Self {
            mem,
            outbox: VecDeque::new(),
            written: 0,
            queued: 0,
        }
    }

    /// Queue a message and write as much as fits now.
    pub fn send(&mut self, msg: &[u8]) {
        let mut framed = Vec::with_capacity(4 + msg.len());
        framed.extend_from_slice(&(msg.len() as u32).to_le_bytes());
        framed.extend_from_slice(msg);
        self.queued += framed.len();
        self.outbox.push_back(framed);
        self.flush();
    }

    /// Write queued bytes while there is space. Returns `true` when the outbox is empty.
    pub fn flush(&mut self) -> bool {
        let mut head = self.mem.load_head();
        while let Some(front) = self.outbox.front() {
            let free = free_space(&self.mem);
            if free == 0 {
                break;
            }
            let n = (front.len() - self.written).min(free);
            put(&self.mem, head, &front[self.written..self.written + n]);
            head = head.wrapping_add(n as u32);
            self.mem.store_head(head);
            self.written += n;
            self.queued -= n;
            if self.written == front.len() {
                self.outbox.pop_front();
                self.written = 0;
            }
        }
        self.outbox.is_empty()
    }

    /// Write the whole message now if it fits and nothing is queued; otherwise drop it and
    /// return `false`. Never allocates.
    pub fn try_send_now(&mut self, msg: &[u8]) -> bool {
        if !self.outbox.is_empty() || free_space(&self.mem) < 4 + msg.len() {
            return false;
        }
        let head = self.mem.load_head();
        put(&self.mem, head, &(msg.len() as u32).to_le_bytes());
        put(&self.mem, head.wrapping_add(4), msg);
        self.mem.store_head(head.wrapping_add(4 + msg.len() as u32));
        true
    }

    /// Bytes queued locally but not yet in the ring.
    pub fn pending_bytes(&self) -> usize {
        self.queued
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RingError {
    #[error("message length {0} exceeds the maximum")]
    TooLarge(usize),
}

/// Consumer half. Reassembles messages across drains; the body buffer is reused, so
/// steady-state reads of small messages don't allocate.
pub struct RingReader<M: RingMemory> {
    mem: M,
    len_buf: [u8; 4],
    len_got: usize,
    body: Vec<u8>,
    /// Expected body length once the prefix is complete.
    body_len: Option<usize>,
}

impl<M: RingMemory> RingReader<M> {
    pub fn new(mem: M) -> Self {
        Self::with_capacity(mem, 0)
    }

    /// Pre-reserve the body buffer (avoid allocations on an RT consumer).
    pub fn with_capacity(mem: M, body_capacity: usize) -> Self {
        Self {
            mem,
            len_buf: [0; 4],
            len_got: 0,
            body: Vec::with_capacity(body_capacity),
            body_len: None,
        }
    }

    /// Bytes available to read.
    pub fn available(&self) -> usize {
        self.mem.load_head().wrapping_sub(self.mem.load_tail()) as usize
    }

    /// Consume up to `budget` bytes, calling `on_message` for each completed message.
    /// Returns the number of bytes consumed. On a corrupt length the ring is emptied and an
    /// error returned (the stream is resynchronized at the current head).
    pub fn drain(
        &mut self,
        budget: usize,
        mut on_message: impl FnMut(&[u8]),
    ) -> Result<usize, RingError> {
        let head = self.mem.load_head();
        let mut tail = self.mem.load_tail();
        let mut consumed = 0;
        loop {
            let avail = head.wrapping_sub(tail) as usize;
            let room = budget - consumed;
            if avail == 0 || room == 0 {
                break;
            }
            match self.body_len {
                None => {
                    let n = (4 - self.len_got).min(avail).min(room);
                    let (got, buf) = (self.len_got, &mut self.len_buf);
                    get(&self.mem, tail, &mut buf[got..got + n]);
                    self.len_got += n;
                    tail = tail.wrapping_add(n as u32);
                    consumed += n;
                    if self.len_got == 4 {
                        let len = u32::from_le_bytes(self.len_buf) as usize;
                        self.len_got = 0;
                        if len > MAX_MESSAGE {
                            self.body.clear();
                            self.mem.store_tail(head);
                            return Err(RingError::TooLarge(len));
                        }
                        self.body.clear();
                        self.body.reserve(len);
                        self.body_len = Some(len);
                    }
                }
                Some(len) => {
                    let have = self.body.len();
                    let n = (len - have).min(avail).min(room);
                    self.body.resize(have + n, 0);
                    get(&self.mem, tail, &mut self.body[have..]);
                    tail = tail.wrapping_add(n as u32);
                    consumed += n;
                }
            }
            if let Some(len) = self.body_len
                && self.body.len() == len
            {
                // Publish the tail before the callback so the producer can refill meanwhile.
                self.mem.store_tail(tail);
                self.body_len = None;
                on_message(&self.body);
                self.body.clear();
            }
        }
        self.mem.store_tail(tail);
        Ok(consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(cap: usize) -> (RingWriter<HeapMemory>, RingReader<HeapMemory>) {
        let mem = HeapMemory::new(cap);
        (RingWriter::new(mem.clone()), RingReader::new(mem))
    }

    fn drain_all(r: &mut RingReader<HeapMemory>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        r.drain(usize::MAX, |m| out.push(m.to_vec())).unwrap();
        out
    }

    #[test]
    fn small_messages_roundtrip_in_order() {
        let (mut w, mut r) = pair(64);
        w.send(b"hello");
        w.send(b"");
        w.send(b"world!");
        assert_eq!(
            drain_all(&mut r),
            vec![b"hello".to_vec(), vec![], b"world!".to_vec()]
        );
        assert_eq!(r.available(), 0);
    }

    #[test]
    fn wraps_around_many_times() {
        let (mut w, mut r) = pair(32);
        for i in 0..1000u32 {
            let msg: Vec<u8> = (0..(i % 13)).map(|j| (i + j) as u8).collect();
            w.send(&msg);
            assert!(w.flush());
            assert_eq!(drain_all(&mut r), vec![msg]);
        }
    }

    #[test]
    fn message_larger_than_ring_streams_across_flushes() {
        let (mut w, mut r) = pair(64);
        let big: Vec<u8> = (0..10_000u32).map(|i| (i * 7) as u8).collect();
        w.send(b"before");
        w.send(&big);
        w.send(b"after");
        let mut got = Vec::new();
        let mut rounds = 0;
        while got.len() < 3 {
            r.drain(usize::MAX, |m| got.push(m.to_vec())).unwrap();
            w.flush();
            rounds += 1;
            assert!(rounds < 10_000, "no progress");
        }
        assert!(rounds > 100);
        assert_eq!(got, vec![b"before".to_vec(), big, b"after".to_vec()]);
        assert_eq!(w.pending_bytes(), 0);
    }

    #[test]
    fn budget_limits_consumption_and_resumes() {
        let (mut w, mut r) = pair(256);
        w.send(&[1; 100]);
        w.send(&[2; 100]);
        let mut got = Vec::new();
        let n = r.drain(50, |m| got.push(m.to_vec())).unwrap();
        assert_eq!(n, 50);
        assert!(got.is_empty());
        r.drain(usize::MAX, |m| got.push(m.to_vec())).unwrap();
        assert_eq!(got, vec![vec![1; 100], vec![2; 100]]);
    }

    #[test]
    fn try_send_now_drops_when_full() {
        let (mut w, mut r) = pair(16);
        assert!(w.try_send_now(&[1; 8]));
        assert!(!w.try_send_now(&[2; 8]), "only 4 bytes left");
        assert_eq!(drain_all(&mut r), vec![vec![1; 8]]);
        assert!(w.try_send_now(&[3; 12]));
        assert_eq!(drain_all(&mut r), vec![vec![3; 12]]);
    }

    #[test]
    fn try_send_now_respects_queued_messages() {
        let (mut w, mut r) = pair(16);
        w.send(&[9; 40]);
        assert!(!w.try_send_now(&[1]), "must not overtake queued bytes");
        let mut got = Vec::new();
        while got.is_empty() {
            r.drain(usize::MAX, |m| got.push(m.to_vec())).unwrap();
            w.flush();
        }
        assert_eq!(got, vec![vec![9; 40]]);
    }

    #[test]
    fn corrupt_length_resyncs() {
        let mem = HeapMemory::new(64);
        mem.write(0, &u32::MAX.to_le_bytes());
        mem.store_head(4);
        let mut r = RingReader::new(mem.clone());
        assert!(r.drain(usize::MAX, |_| {}).is_err());
        assert_eq!(r.available(), 0);
        let mut w = RingWriter::new(mem);
        w.send(b"ok");
        assert_eq!(drain_all(&mut r), vec![b"ok".to_vec()]);
    }

    #[test]
    fn concurrent_producer_consumer() {
        let mem = HeapMemory::new(128);
        let mut w = RingWriter::new(mem.clone());
        let mut r = RingReader::new(mem);
        let producer = std::thread::spawn(move || {
            for i in 0..2000u32 {
                let msg: Vec<u8> = (0..(i % 300)).map(|j| (i ^ j) as u8).collect();
                w.send(&msg);
                while !w.flush() {
                    std::thread::yield_now();
                }
            }
        });
        let mut expected = 0u32;
        while expected < 2000 {
            r.drain(usize::MAX, |m| {
                let want: Vec<u8> = (0..(expected % 300))
                    .map(|j| (expected ^ j) as u8)
                    .collect();
                assert_eq!(m, &want[..]);
                expected += 1;
            })
            .unwrap();
            std::thread::yield_now();
        }
        producer.join().unwrap();
    }
}
