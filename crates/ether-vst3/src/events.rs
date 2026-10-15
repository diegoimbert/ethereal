//! Pre-allocated `IParameterChanges` / `IParamValueQueue` / `IEventList` for `ProcessData`.
//!
//! Everything is allocated up front (non-RT); on the audio thread the host only clears and
//! fills them, and the plugin reads (or, for the output ones, writes) them during `process`.
//! Fixed capacities: extra params/points/events are dropped rather than allocated.
//!
//! Interior mutability is an `UnsafeCell`: the host touches these objects only on the audio
//! thread, outside `IAudioProcessor::process`, and the plugin only inside it, so accesses
//! never overlap.
//!
//! **Point pool** (perf-plugins, audit #16): the points of every queue live in one pool
//! shared by the block's queues, sized for a block's worth of points, instead of a
//! worst-case buffer per param (`params × (max_events + 1)` points, ~33 MB for a
//! 2048-param plugin). Points are appended as they come; the first read sorts the pool by
//! (queue, offset) in place, keeping the last point of equal offsets (a point at an
//! existing offset replaces it), so each queue reads one contiguous run. Queue lookup by
//! param id is a pre-sized hash map (no linear scan, no allocation on the audio thread).

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::Arc;

use vst3::Steinberg::Vst::{
    Event, IEventList, IEventListTrait, IParamValueQueue, IParamValueQueueTrait, IParameterChanges,
    IParameterChangesTrait, ParamID, ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComWrapper};

/// One point of a [`ParamChanges`] pool.
#[derive(Clone, Copy, Debug)]
struct Point {
    /// Index of the owning queue in `ParamChanges::queues`.
    queue: u32,
    offset: int32,
    /// Insertion order (the last of equal offsets wins).
    seq: u32,
    value: ParamValue,
}

/// The shared state of one [`ParamChanges`]: its point pool and active queues.
struct Pool {
    points: Vec<Point>,
    /// `points` is not sorted/deduplicated since the last append.
    dirty: bool,
    /// Per queue: `(start, len)` into `points` once sorted; while dirty, `len` counts the
    /// points appended (duplicates included).
    runs: Vec<(usize, usize)>,
    /// Per queue: its last point by (offset, seq), for `for_each_last` without sorting.
    last: Vec<Option<(int32, u32, ParamValue)>>,
    /// Param id → queue index for the queues in use.
    index: HashMap<ParamID, u32>,
    /// Param id per queue in use (`used` = its length).
    ids: Vec<ParamID>,
    seq: u32,
}

impl Pool {
    /// Sort and deduplicate `points` (RT: in place, no allocation) and recompute the runs.
    fn settle(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.points
            .sort_unstable_by_key(|p| (p.queue, p.offset, p.seq));
        // Keep the last of each (queue, offset).
        let mut w = 0;
        for r in 0..self.points.len() {
            let p = self.points[r];
            if r + 1 < self.points.len() {
                let n = self.points[r + 1];
                if n.queue == p.queue && n.offset == p.offset {
                    continue;
                }
            }
            self.points[w] = p;
            w += 1;
        }
        self.points.truncate(w);
        for run in &mut self.runs[..self.ids.len()] {
            *run = (0, 0);
        }
        let mut i = 0;
        while i < self.points.len() {
            let q = self.points[i].queue as usize;
            let start = i;
            while i < self.points.len() && self.points[i].queue as usize == q {
                i += 1;
            }
            self.runs[q] = (start, i - start);
        }
    }

    /// Append a point to queue `q`; `false` when the pool is full.
    fn push(&mut self, q: u32, offset: int32, value: ParamValue) -> bool {
        if self.points.len() == self.points.capacity() {
            return false;
        }
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        self.points.push(Point {
            queue: q,
            offset,
            seq,
            value,
        });
        self.dirty = true;
        self.runs[q as usize].1 += 1;
        let last = &mut self.last[q as usize];
        if last.is_none_or(|(o, s, _)| (o, s) <= (offset, seq)) {
            *last = Some((offset, seq, value));
        }
        true
    }

    /// Point `index` of queue `q` (in offset order).
    fn point(&mut self, q: u32, index: usize) -> Option<(int32, ParamValue)> {
        self.settle();
        let (start, len) = self.runs[q as usize];
        (index < len).then(|| {
            let p = self.points[start + index];
            (p.offset, p.value)
        })
    }

    fn count(&mut self, q: u32) -> usize {
        self.settle();
        self.runs[q as usize].1
    }
}

/// Shared handle to a [`Pool`] (one per [`ParamChanges`], cloned into its queues).
struct PoolCell(UnsafeCell<Pool>);

// SAFETY: see the module docs (accesses are serialized by the process call protocol).
unsafe impl Sync for PoolCell {}
// SAFETY: moved between threads only as part of the node, never accessed concurrently.
unsafe impl Send for PoolCell {}

impl PoolCell {
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut Pool {
        // SAFETY: see the module docs.
        unsafe { &mut *self.0.get() }
    }
}

/// One parameter's points in a block, sorted by sample offset (a view of the pool).
pub(crate) struct ParamQueue {
    pool: Arc<PoolCell>,
    /// This queue's index in the pool.
    slot: u32,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl ParamQueue {
    fn id(&self) -> ParamID {
        self.pool.get().ids[self.slot as usize]
    }
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.pool.get().count(self.slot) as int32
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        let Some((o, v)) = usize::try_from(index)
            .ok()
            .and_then(|i| self.pool.get().point(self.slot, i))
        else {
            return kInvalidArgument;
        };
        if offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked non-null out pointers.
        unsafe {
            *offset = o;
            *value = v;
        }
        kResultOk
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let pool = self.pool.get();
        if !pool.push(self.slot, offset, value) {
            return kResultFalse;
        }
        if !index.is_null() {
            // The point's index among its queue's points in offset order.
            pool.settle();
            let (start, len) = pool.runs[self.slot as usize];
            let i = pool.points[start..start + len].partition_point(|p| p.offset < offset);
            // SAFETY: checked non-null out pointer.
            unsafe { *index = i as int32 };
        }
        kResultOk
    }
}

/// A block's parameter changes: up to `params` queues sharing one point pool.
pub(crate) struct ParamChanges {
    // Declared before `pool`: the queues are dropped first.
    queues: Vec<ComWrapper<ParamQueue>>,
    pool: Arc<PoolCell>,
}

// SAFETY: see the module docs.
unsafe impl Sync for ParamChanges {}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl ParamChanges {
    /// Non-RT: up to `params` queues sharing a pool of `points` points.
    pub fn new(params: usize, points: usize) -> ComWrapper<Self> {
        let params = params.max(1);
        let pool = Arc::new(PoolCell(UnsafeCell::new(Pool {
            points: Vec::with_capacity(points.max(1)),
            dirty: false,
            runs: vec![(0, 0); params],
            last: vec![None; params],
            index: HashMap::with_capacity(params),
            ids: Vec::with_capacity(params),
            seq: 0,
        })));
        ComWrapper::new(Self {
            queues: (0..params)
                .map(|slot| {
                    ComWrapper::new(ParamQueue {
                        pool: pool.clone(),
                        slot: slot as u32,
                    })
                })
                .collect(),
            pool,
        })
    }

    /// Bytes held by this object's buffers (tests, memory budget).
    #[cfg(test)]
    pub fn footprint(&self) -> usize {
        let p = self.pool.get();
        p.points.capacity() * std::mem::size_of::<Point>()
            + p.runs.capacity() * std::mem::size_of::<(usize, usize)>()
            + p.last.capacity() * std::mem::size_of::<Option<(int32, u32, ParamValue)>>()
            + p.index.capacity() * (std::mem::size_of::<(ParamID, u32)>() + 1)
            + p.ids.capacity() * std::mem::size_of::<ParamID>()
            + self.queues.len() * std::mem::size_of::<ParamQueue>()
    }

    fn used(&self) -> usize {
        self.pool.get().ids.len()
    }

    /// RT: drop every queue's points.
    pub fn clear(&self) {
        let p = self.pool.get();
        for q in 0..p.ids.len() {
            p.runs[q] = (0, 0);
            p.last[q] = None;
        }
        p.points.clear();
        p.index.clear();
        p.ids.clear();
        p.dirty = false;
        p.seq = 0;
    }

    /// The queue of `id` (claimed from the free ones if new), `None` when all are in use.
    fn queue_for(&self, id: ParamID) -> Option<u32> {
        let p = self.pool.get();
        if let Some(&q) = p.index.get(&id) {
            return Some(q);
        }
        if p.ids.len() == self.queues.len() {
            return None;
        }
        let q = p.ids.len() as u32;
        p.ids.push(id);
        p.index.insert(id, q);
        Some(q)
    }

    /// RT: add a point for `id` (dropped if the queues or the pool are full).
    pub fn add(&self, id: ParamID, offset: u32, value: ParamValue) -> bool {
        match self.queue_for(id) {
            Some(q) => self.pool.get().push(q, offset as int32, value),
            None => false,
        }
    }

    /// RT: (id, last value) of every queue.
    pub fn for_each_last(&self, mut f: impl FnMut(ParamID, ParamValue)) {
        let p = self.pool.get();
        for q in 0..p.ids.len() {
            if let Some((_, _, v)) = p.last[q] {
                f(p.ids[q], v);
            }
        }
    }
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.used() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let Some(q) = usize::try_from(index)
            .ok()
            .filter(|i| *i < self.used())
            .map(|i| &self.queues[i])
        else {
            return std::ptr::null_mut();
        };
        // Not add-ref'd (SDK semantics: owned by the changes object).
        q.as_com_ref::<IParamValueQueue>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: checked non-null.
        let Some(q) = self.queue_for(unsafe { *id }) else {
            return std::ptr::null_mut();
        };
        if !index.is_null() {
            // SAFETY: checked non-null out pointer.
            unsafe { *index = q as int32 };
        }
        self.queues[q as usize]
            .as_com_ref::<IParamValueQueue>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

/// A block's events (notes), in the order added (the host adds them sorted by offset).
pub(crate) struct EventList {
    events: UnsafeCell<Vec<Event>>,
}

// SAFETY: see the module docs.
unsafe impl Sync for EventList {}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl EventList {
    pub fn new(capacity: usize) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            events: UnsafeCell::new(Vec::with_capacity(capacity.max(1))),
        })
    }

    #[allow(clippy::mut_from_ref)]
    fn events(&self) -> &mut Vec<Event> {
        // SAFETY: see the module docs.
        unsafe { &mut *self.events.get() }
    }

    pub fn clear(&self) {
        self.events().clear();
    }

    /// RT: add an event (dropped when full).
    pub fn push(&self, e: Event) -> bool {
        let events = self.events();
        if events.len() == events.capacity() {
            return false;
        }
        events.push(e);
        true
    }

    pub fn len(&self) -> usize {
        self.events().len()
    }

    pub fn get(&self, i: usize) -> Option<&Event> {
        self.events().get(i)
    }
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events().len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        let Some(ev) = usize::try_from(index)
            .ok()
            .and_then(|i| self.events().get(i))
        else {
            return kInvalidArgument;
        };
        if e.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked non-null out pointer.
        unsafe { *e = *ev };
        kResultOk
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked non-null.
        if self.push(unsafe { *e }) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_changes_pool() {
        // 2 queues sharing 4 points.
        let changes = ParamChanges::new(2, 4);
        let p = changes.to_com_ptr::<IParameterChanges>().unwrap();
        assert!(changes.add(7, 10, 0.5));
        assert!(changes.add(7, 3, 0.25));
        assert!(changes.add(7, 10, 0.75)); // same offset: replaced
        assert!(changes.add(8, 0, 1.0));
        assert!(!changes.add(7, 20, 1.0)); // pool full
        assert!(!changes.add(9, 0, 1.0)); // no free queue
        unsafe {
            assert_eq!(p.getParameterCount(), 2);
            let q = vst3::ComRef::from_raw(p.getParameterData(0)).unwrap();
            assert_eq!(q.getParameterId(), 7);
            assert_eq!(q.getPointCount(), 2);
            let (mut o, mut v) = (0, 0.0);
            assert_eq!(q.getPoint(0, &mut o, &mut v), kResultOk);
            assert_eq!((o, v), (3, 0.25));
            assert_eq!(q.getPoint(1, &mut o, &mut v), kResultOk);
            assert_eq!((o, v), (10, 0.75));
            assert_eq!(q.getPoint(2, &mut o, &mut v), kInvalidArgument);
            assert!(p.getParameterData(2).is_null());
        }
        let mut last = Vec::new();
        changes.for_each_last(|id, v| last.push((id, v)));
        assert_eq!(last, [(7, 0.75), (8, 1.0)]);
        changes.clear();
        unsafe {
            assert_eq!(p.getParameterCount(), 0);
            let id = 5;
            let mut index = -1;
            let q = vst3::ComRef::from_raw(p.addParameterData(&id, &mut index)).unwrap();
            assert_eq!(index, 0);
            assert_eq!(q.getPointCount(), 0);
            q.addPoint(4, 0.5, std::ptr::null_mut());
            assert_eq!(p.getParameterCount(), 1);
        }
    }

    /// perf-plugins (audit #16): one shared pool instead of `params × (max_events + 1)`
    /// points; one param may still use the whole pool, and lookups are by id.
    #[test]
    fn param_changes_share_one_pool() {
        let (params, max_events) = (2048, 1024);
        let changes = ParamChanges::new(params, 2 * (max_events + 1) + params);
        // Was 2048 × 1025 × 16 B ≈ 33.6 MB of point storage.
        eprintln!(
            "VST3 input param changes, 2048 params: {} bytes",
            changes.footprint()
        );
        assert!(
            changes.footprint() < 1 << 20,
            "{} bytes",
            changes.footprint()
        );
        let p = changes.to_com_ptr::<IParameterChanges>().unwrap();
        // A preset recall: every param at offset 0, plus dense automation on one param.
        for id in 0..params as u32 {
            assert!(changes.add(1000 + id, 0, 0.5));
        }
        for i in (0..1024u32).rev() {
            assert!(changes.add(1000 + 5, i, f64::from(i) / 1024.0));
        }
        unsafe {
            assert_eq!(p.getParameterCount(), params as i32);
            let q = vst3::ComRef::from_raw(p.getParameterData(5)).unwrap();
            assert_eq!(q.getParameterId(), 1005);
            assert_eq!(q.getPointCount(), 1024); // offset 0 replaced by the later point
            let (mut o, mut v) = (0, 0.0);
            for i in 0..1024 {
                assert_eq!(q.getPoint(i, &mut o, &mut v), kResultOk);
                assert_eq!((o, v), (i, f64::from(i) / 1024.0));
            }
            let q = vst3::ComRef::from_raw(p.getParameterData(6)).unwrap();
            assert_eq!(q.getPointCount(), 1);
        }
        let mut last = 0;
        changes.for_each_last(|id, v| {
            if id == 1005 {
                assert_eq!(v, 1023.0 / 1024.0);
            }
            last += 1;
        });
        assert_eq!(last, params);
        changes.clear();
        unsafe { assert_eq!(p.getParameterCount(), 0) };
    }

    #[test]
    fn event_list_capacity() {
        let list = EventList::new(1);
        let p = list.to_com_ptr::<IEventList>().unwrap();
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.sampleOffset = 5;
        unsafe {
            assert_eq!(p.addEvent(&mut e), kResultOk);
            assert_eq!(p.addEvent(&mut e), kResultFalse);
            assert_eq!(p.getEventCount(), 1);
            let mut out: Event = std::mem::zeroed();
            assert_eq!(p.getEvent(0, &mut out), kResultOk);
            assert_eq!(out.sampleOffset, 5);
        }
        list.clear();
        assert_eq!(list.len(), 0);
    }
}
