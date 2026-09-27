//! Pre-allocated `IParameterChanges` / `IParamValueQueue` / `IEventList` for `ProcessData`.
//!
//! Everything is allocated up front (non-RT); on the audio thread the host only clears and
//! fills them, and the plugin reads (or, for the output ones, writes) them during `process`.
//! Fixed capacities: extra params/points/events are dropped rather than allocated.
//!
//! Interior mutability is an `UnsafeCell`: the host touches these objects only on the audio
//! thread, outside `IAudioProcessor::process`, and the plugin only inside it, so accesses
//! never overlap.

use std::cell::UnsafeCell;

use vst3::Steinberg::Vst::{
    Event, IEventList, IEventListTrait, IParamValueQueue, IParamValueQueueTrait,
    IParameterChanges, IParameterChangesTrait, ParamID, ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComWrapper};

struct QueueInner {
    id: ParamID,
    points: Vec<(int32, ParamValue)>,
}

/// One parameter's points in a block, sorted by sample offset.
pub(crate) struct ParamQueue {
    inner: UnsafeCell<QueueInner>,
}

// SAFETY: see the module docs (accesses are serialized by the process call protocol).
unsafe impl Sync for ParamQueue {}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl ParamQueue {
    fn new(capacity: usize) -> Self {
        Self {
            inner: UnsafeCell::new(QueueInner {
                id: 0,
                points: Vec::with_capacity(capacity),
            }),
        }
    }

    #[allow(clippy::mut_from_ref)]
    fn inner(&self) -> &mut QueueInner {
        // SAFETY: see the module docs.
        unsafe { &mut *self.inner.get() }
    }

    /// Add a point, keeping offsets sorted (a point at an existing offset replaces it).
    /// Returns the point index, or `None` when full.
    fn add(&self, offset: int32, value: ParamValue) -> Option<int32> {
        let points = &mut self.inner().points;
        let at = points.partition_point(|(o, _)| *o < offset);
        if points.get(at).is_some_and(|(o, _)| *o == offset) {
            points[at].1 = value;
            return Some(at as int32);
        }
        if points.len() == points.capacity() {
            return None;
        }
        points.insert(at, (offset, value));
        Some(at as int32)
    }

    fn last(&self) -> Option<ParamValue> {
        self.inner().points.last().map(|(_, v)| *v)
    }
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.inner().id
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.inner().points.len() as int32
    }

    unsafe fn getPoint(
        &self,
        index: int32,
        offset: *mut int32,
        value: *mut ParamValue,
    ) -> tresult {
        let Some(&(o, v)) = usize::try_from(index)
            .ok()
            .and_then(|i| self.inner().points.get(i))
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
        match self.add(offset, value) {
            Some(i) => {
                if !index.is_null() {
                    // SAFETY: checked non-null out pointer.
                    unsafe { *index = i };
                }
                kResultOk
            }
            None => kResultFalse,
        }
    }
}

struct ChangesInner {
    used: usize,
}

/// A block's parameter changes: a fixed pool of queues, `used` of them active.
pub(crate) struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    inner: UnsafeCell<ChangesInner>,
}

// SAFETY: see the module docs.
unsafe impl Sync for ParamChanges {}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl ParamChanges {
    /// Non-RT: `params` queues of `points` points each.
    pub fn new(params: usize, points: usize) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            queues: (0..params.max(1))
                .map(|_| ComWrapper::new(ParamQueue::new(points.max(1))))
                .collect(),
            inner: UnsafeCell::new(ChangesInner { used: 0 }),
        })
    }

    fn used(&self) -> usize {
        // SAFETY: see the module docs.
        unsafe { (*self.inner.get()).used }
    }

    fn set_used(&self, n: usize) {
        // SAFETY: see the module docs.
        unsafe { (*self.inner.get()).used = n }
    }

    /// RT: drop every queue's points.
    pub fn clear(&self) {
        for q in &self.queues[..self.used()] {
            q.inner().points.clear();
        }
        self.set_used(0);
    }

    fn queue_for(&self, id: ParamID) -> Option<(usize, &ComWrapper<ParamQueue>)> {
        let used = self.used();
        if let Some(i) = self.queues[..used].iter().position(|q| q.inner().id == id) {
            return Some((i, &self.queues[i]));
        }
        let q = self.queues.get(used)?;
        q.inner().id = id;
        q.inner().points.clear();
        self.set_used(used + 1);
        Some((used, q))
    }

    /// RT: add a point for `id` (dropped if the pool or the queue is full).
    pub fn add(&self, id: ParamID, offset: u32, value: ParamValue) -> bool {
        match self.queue_for(id) {
            Some((_, q)) => q.add(offset as int32, value).is_some(),
            None => false,
        }
    }

    /// RT: (id, last value) of every queue.
    pub fn for_each_last(&self, mut f: impl FnMut(ParamID, ParamValue)) {
        for q in &self.queues[..self.used()] {
            if let Some(v) = q.last() {
                f(q.inner().id, v);
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

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: checked non-null.
        let Some((i, q)) = self.queue_for(unsafe { *id }) else {
            return std::ptr::null_mut();
        };
        if !index.is_null() {
            // SAFETY: checked non-null out pointer.
            unsafe { *index = i as int32 };
        }
        q.as_com_ref::<IParamValueQueue>()
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
        let changes = ParamChanges::new(2, 2);
        let p = changes.to_com_ptr::<IParameterChanges>().unwrap();
        assert!(changes.add(7, 10, 0.5));
        assert!(changes.add(7, 3, 0.25));
        assert!(changes.add(7, 10, 0.75)); // same offset: replaced
        assert!(!changes.add(7, 20, 1.0)); // queue full
        assert!(changes.add(8, 0, 1.0));
        assert!(!changes.add(9, 0, 1.0)); // pool full
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
