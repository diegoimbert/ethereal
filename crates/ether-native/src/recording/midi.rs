//! MIDI input ports (`midir`): every port is connected; short messages go to the engine
//! (live MIDI) and, tagged with their port, to the controller ([`ControlQueue`], drained by
//! `EngineBridge::poll_midi_input` for MIDI learn / mappings).
//!
//! The controller path of a midir callback neither locks nor allocates: each connected port
//! owns the producer of its own SPSC ring ([`ControlPort`]), and its name is interned once,
//! when the port is connected. The controller thread drains every ring and restores the
//! arrival order across ports from a global sequence number.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ether_core::protocol::midi_map::MidiInputEvent;
use ether_core::protocol::recording::MidiPort;
use midir::{Ignore, MidiInput, MidiInputConnection};
use rtrb::{Consumer, Producer, RingBuffer};

const CLIENT: &str = "Ethereal";

/// Most messages kept per port for the controller between two ticks. When a port's ring is
/// full its newest messages are dropped (a controller that stops ticking must not grow
/// memory, and a producer cannot evict from the consumer side).
pub(super) const CONTROL_QUEUE_CAPACITY: usize = 1024;

/// One message in a port's ring.
#[derive(Clone, Copy, Debug)]
struct Raw {
    /// Arrival order across all ports.
    seq: u64,
    time_ms: f64,
    data: [u8; 3],
}

/// The producer side of one port's ring, owned by that port's midir callback.
pub(super) struct ControlPort {
    tx: Producer<Raw>,
    seq: Arc<AtomicU64>,
    /// [`ether_core::recording::port_key`] of the port's name, computed once (tags live MIDI
    /// for the tracks' port filters).
    key: u64,
}

impl ControlPort {
    /// Queue a message for the controller. Lock- and allocation-free; `false` if the ring
    /// is full (the message is dropped).
    pub fn push(&mut self, data: [u8; 3]) -> bool {
        let time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        self.tx.push(Raw { seq, time_ms, data }).is_ok()
    }
}

#[derive(Default)]
struct Consumers {
    /// Interned port names (one allocation per name, reused when a port reconnects).
    names: Vec<Arc<str>>,
    /// The consumer of every registered ring, with its port's interned name.
    rings: Vec<(Arc<str>, Consumer<Raw>)>,
    /// Drain scratch: messages and their ring index, sorted by `seq`.
    scratch: Vec<(Raw, usize)>,
}

/// Incoming messages for the controller (MIDI learn / mappings): one SPSC ring per port,
/// written by its midir callback thread and drained by the controller thread. The lock is
/// taken when a port is registered and by the drain, never by a callback.
#[derive(Default)]
pub(super) struct ControlQueue {
    seq: Arc<AtomicU64>,
    consumers: Mutex<Consumers>,
    /// Producers of [`ControlQueue::inject`] (tests, virtual inputs), by port name.
    injected: Mutex<HashMap<Arc<str>, ControlPort>>,
}

impl std::fmt::Debug for ControlQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlQueue").finish_non_exhaustive()
    }
}

impl ControlQueue {
    /// A new ring for port `name`; its producer goes to the port's callback. The ring is
    /// forgotten once its producer is dropped and it has been drained.
    pub fn register(&self, name: &str) -> ControlPort {
        let (tx, rx) = RingBuffer::new(CONTROL_QUEUE_CAPACITY);
        let mut c = self.consumers.lock().unwrap_or_else(|p| p.into_inner());
        let name = match c.names.iter().find(|n| &***n == name) {
            Some(n) => n.clone(),
            None => {
                let n: Arc<str> = Arc::from(name);
                c.names.push(n.clone());
                n
            }
        };
        let key = ether_core::recording::port_key(&name);
        c.rings.push((name, rx));
        ControlPort {
            tx,
            seq: self.seq.clone(),
            key,
        }
    }

    /// Queue a message as if port `name` had sent it. Not for midir callbacks: it takes a
    /// lock, and registers the port on first use.
    pub fn inject(&self, name: &str, data: [u8; 3]) -> bool {
        let mut injected = self.injected.lock().unwrap_or_else(|p| p.into_inner());
        if !injected.contains_key(name) {
            let port = self.register(name);
            injected.insert(Arc::from(name), port);
        }
        injected.get_mut(name).is_some_and(|p| p.push(data))
    }

    /// Every queued message, in arrival order (controller thread).
    pub fn drain(&self, out: &mut Vec<MidiInputEvent>) {
        let mut guard = self.consumers.lock().unwrap_or_else(|p| p.into_inner());
        let c = &mut *guard;
        c.scratch.clear();
        for (i, (_, rx)) in c.rings.iter_mut().enumerate() {
            while let Ok(raw) = rx.pop() {
                c.scratch.push((raw, i));
            }
        }
        c.scratch.sort_unstable_by_key(|(raw, _)| raw.seq);
        out.extend(c.scratch.iter().map(|(raw, i)| MidiInputEvent {
            port: c.rings[*i].0.to_string(),
            data: raw.data,
            time_ms: raw.time_ms,
        }));
        // Closed ports (connection dropped): nothing more can arrive.
        c.rings
            .retain(|(_, rx)| !(rx.is_abandoned() && rx.is_empty()));
    }
}

/// A port's midir callback: live MIDI to `on_live` (with the port's key), then the message
/// to the port's ring.
fn input_callback(
    on_live: Arc<dyn Fn(u64, [u8; 3]) + Send + Sync>,
    mut port: ControlPort,
) -> impl FnMut(u64, &[u8], &mut ()) + Send + 'static {
    move |_stamp, bytes, _| {
        if let Some(msg) = short_message(bytes) {
            on_live(port.key, msg);
            port.push(msg);
        }
    }
}

/// Connected MIDI input ports (by name).
#[derive(Default)]
pub(super) struct MidiInputs {
    connections: Vec<(String, MidiInputConnection<()>)>,
}

impl MidiInputs {
    /// Connect ports that appeared since the last call (and forget vanished ones); returns
    /// every available port and the connections of vanished ports (to close). Each newly
    /// connected port gets a ring in `control`; `on_live` receives the port's key and short
    /// messages (status + up to 2 data bytes) on midir's thread.
    pub fn refresh(
        &mut self,
        control: &ControlQueue,
        on_live: impl Fn(u64, [u8; 3]) + Send + Sync + 'static,
    ) -> (Vec<MidiPort>, Vec<MidiInputConnection<()>>) {
        let Ok(probe) = MidiInput::new(CLIENT) else {
            return (Vec::new(), Vec::new());
        };
        let ports: Vec<(String, midir::MidiInputPort)> = probe
            .ports()
            .into_iter()
            .filter_map(|p| Some((probe.port_name(&p).ok()?, p)))
            .collect();
        // Vanished ports: returned to the caller, which closes them outside any lock.
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.connections)
            .into_iter()
            .partition(|(name, _)| ports.iter().any(|(n, _)| n == name));
        self.connections = keep;
        let gone = gone.into_iter().map(|(_, c)| c).collect();
        let on_live: Arc<dyn Fn(u64, [u8; 3]) + Send + Sync> = Arc::new(on_live);
        for (name, port) in &ports {
            if self.connections.iter().any(|(n, _)| n == name) {
                continue;
            }
            let Ok(mut input) = MidiInput::new(CLIENT) else {
                continue;
            };
            input.ignore(Ignore::All);
            let cb = input_callback(on_live.clone(), control.register(name));
            match input.connect(port, "ethereal-in", cb, ()) {
                Ok(conn) => self.connections.push((name.clone(), conn)),
                Err(e) => tracing::warn!(port = %name, error = %e, "could not open MIDI input"),
            }
        }
        let ports = ports
            .into_iter()
            .map(|(name, _)| MidiPort {
                id: name.clone(),
                name,
            })
            .collect();
        (ports, gone)
    }
}

/// A channel voice message as 3 bytes (`None` for system/realtime/SysEx data).
pub(super) fn short_message(bytes: &[u8]) -> Option<[u8; 3]> {
    let status = *bytes.first()?;
    if !(0x80..0xf0).contains(&status) {
        return None;
    }
    Some([
        status,
        bytes.get(1).copied().unwrap_or(0),
        bytes.get(2).copied().unwrap_or(0),
    ])
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use super::*;

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ALLOCS: Cell<usize> = const { Cell::new(0) };
    }

    /// Counts the allocations (and frees) of the threads that armed it: the lib's other
    /// tests run concurrently and are not counted.
    struct Counting;

    fn count() {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
            }
        });
    }

    // SAFETY: forwards every call unchanged to `System`; only counts.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count();
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            count();
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            count();
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            count();
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static A: Counting = Counting;

    /// Allocations made by `f` on this thread.
    fn allocations(f: impl FnOnce()) -> usize {
        ALLOCS.with(|n| n.set(0));
        COUNTING.with(|on| on.set(true));
        f();
        COUNTING.with(|on| on.set(false));
        ALLOCS.with(|n| n.get())
    }

    #[test]
    fn the_counting_allocator_counts() {
        assert!(allocations(|| drop(std::hint::black_box(String::from("port")))) >= 1);
    }

    #[test]
    fn the_callback_neither_allocates_nor_locks() {
        let q = ControlQueue::default();
        let live = Arc::new(AtomicU64::new(0));
        let seen = live.clone();
        let mut cb = input_callback(
            Arc::new(move |port, _| {
                assert_eq!(port, ether_core::recording::port_key("Knobs"));
                seen.fetch_add(1, Ordering::Relaxed);
            }),
            q.register("Knobs"),
        );
        // The drain lock held by another thread never blocks the callback.
        let guard = q.consumers.lock().unwrap();
        let n = allocations(|| {
            for i in 0..500u32 {
                cb(u64::from(i), &[0xb0, 21, (i % 128) as u8], &mut ());
                // Ignored (SysEx / clock), still nothing allocated.
                cb(u64::from(i), &[0xf8], &mut ());
            }
        });
        drop(guard);
        assert_eq!(n, 0, "the midir callback allocated");
        assert_eq!(live.load(Ordering::Relaxed), 500);
        let mut out = Vec::new();
        q.drain(&mut out);
        assert_eq!(out.len(), 500);
        assert!(out.iter().all(|e| e.port == "Knobs" && e.time_ms > 0.0));
        assert_eq!(out[499].data, [0xb0, 21, (499 % 128) as u8]);
    }

    #[test]
    fn port_names_are_interned() {
        let q = ControlQueue::default();
        let a = q.register("Port A");
        drop(a);
        // Reconnecting reuses the name; a new name is interned once.
        let _a = q.register("Port A");
        let _b = q.register("Port B");
        let c = q.consumers.lock().unwrap();
        assert_eq!(c.names.len(), 2);
        assert!(Arc::ptr_eq(&c.rings[0].0, &c.rings[1].0));
    }

    #[test]
    fn rings_are_bounded_and_drain_in_arrival_order_across_ports() {
        let q = ControlQueue::default();
        let mut a = q.register("A");
        let mut b = q.register("B");
        for i in 0..CONTROL_QUEUE_CAPACITY + 10 {
            let ok = a.push([0xb0, 1, (i % 128) as u8]);
            assert_eq!(ok, i < CONTROL_QUEUE_CAPACITY);
        }
        let mut out = Vec::new();
        q.drain(&mut out);
        assert_eq!(out.len(), CONTROL_QUEUE_CAPACITY);
        // The newest messages were dropped.
        assert_eq!(out[0].data, [0xb0, 1, 0]);

        out.clear();
        b.push([0x90, 60, 100]);
        a.push([0xb0, 2, 1]);
        b.push([0x80, 60, 0]);
        q.drain(&mut out);
        let order: Vec<_> = out.iter().map(|e| (e.port.as_str(), e.data[0])).collect();
        assert_eq!(order, [("B", 0x90), ("A", 0xb0), ("B", 0x80)]);
        let mut again = Vec::new();
        q.drain(&mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn closed_ports_are_forgotten_after_their_last_messages() {
        let q = ControlQueue::default();
        let mut a = q.register("A");
        a.push([0xb0, 1, 1]);
        drop(a);
        let mut out = Vec::new();
        q.drain(&mut out);
        assert_eq!(out.len(), 1);
        assert!(q.consumers.lock().unwrap().rings.is_empty());
    }

    #[test]
    fn inject_registers_the_port_once() {
        let q = ControlQueue::default();
        assert!(q.inject("Virtual", [0xb0, 7, 1]));
        assert!(q.inject("Virtual", [0xb0, 7, 2]));
        assert_eq!(q.consumers.lock().unwrap().rings.len(), 1);
        let mut out = Vec::new();
        q.drain(&mut out);
        assert_eq!(out.len(), 2);
    }
}
