//! Audio-thread half of an AU instance ([`AuNode`]).
//!
//! Everything is prepared in [`AuNode::new`] (main thread, at activation): the unit's render
//! block, schedule-parameter/MIDI blocks (copied once), the pull-input block and the
//! host-context blocks (created once; they read [`RtState`] through a raw pointer), and the
//! output `AudioBufferList` + buffers. `process` then only fills buffers, invokes blocks
//! and copies: no Rust allocation and no Objective-C message sends on the audio thread
//! (`reset`, which the AU API allows on the render thread, goes through the method's
//! implementation pointer looked up at activation, bypassing dynamic dispatch).
//!
//! Engine events become render events: `Param` → `scheduleParameterBlock`
//! (`AUEventSampleTimeImmediate + offset`, sample accurate), notes/MIDI →
//! `scheduleMIDIEventBlock` (same timing), both issued right before the render call.

use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use block2::{DynBlock, RcBlock};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventKind, ProcessEvent};
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::PluginNode;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;
use ether_core::transport::TransportInfo;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Imp, Sel};
use objc2::sel;
use objc2_audio_toolbox::{
    AUAudioUnit, AUEventSampleTimeImmediate, AUHostTransportStateFlags, AudioUnitRenderActionFlags,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp, AudioTimeStampFlags};

/// Max channels per bus we pass to a unit.
pub(crate) const MAX_CHANNELS: usize = 16;
/// MIDI output events buffered per render call.
const MIDI_OUT_CAP: usize = 512;
/// Consecutive render errors before the node is considered faulted.
const MAX_RENDER_ERRORS: u32 = 64;

pub(crate) type RenderFn = dyn Fn(
    NonNull<AudioUnitRenderActionFlags>,
    NonNull<AudioTimeStamp>,
    u32,
    isize,
    NonNull<AudioBufferList>,
    *mut DynBlock<PullFn>,
) -> i32;
pub(crate) type PullFn = dyn Fn(
    NonNull<AudioUnitRenderActionFlags>,
    NonNull<AudioTimeStamp>,
    u32,
    isize,
    NonNull<AudioBufferList>,
) -> i32;
pub(crate) type ScheduleParamFn = dyn Fn(i64, u32, u64, f32);
pub(crate) type ScheduleMidiFn = dyn Fn(i64, u8, isize, NonNull<u8>);
pub(crate) type MidiOutFn = dyn Fn(i64, u8, isize, NonNull<u8>) -> i32;
pub(crate) type MusicalContextFn =
    dyn Fn(*mut f64, *mut f64, *mut isize, *mut f64, *mut isize, *mut f64) -> Bool;
pub(crate) type TransportStateFn =
    dyn Fn(*mut AUHostTransportStateFlags, *mut f64, *mut f64, *mut f64) -> Bool;

/// `AudioBufferList` with room for [`MAX_CHANNELS`] buffers (same layout prefix).
#[repr(C)]
pub(crate) struct BufList {
    pub count: u32,
    pub buffers: [AudioBuffer; MAX_CHANNELS],
}

impl BufList {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            count: 0,
            buffers: [AudioBuffer {
                mNumberChannels: 1,
                mDataByteSize: 0,
                mData: std::ptr::null_mut(),
            }; MAX_CHANNELS],
        })
    }
}

/// State shared between the node and its controller.
#[derive(Debug, Default)]
pub(crate) struct NodeShared {
    pub faulted: AtomicBool,
    pub latency: AtomicU32,
}

/// Per-param "last value the host scheduled" (f32 bits), so the controller can tell GUI
/// edits from echoes of automation. Indexed like `ParamLinks::entries`.
#[derive(Debug, Default)]
pub(crate) struct ParamLinks {
    /// `(address, id)` sorted by address.
    pub entries: Vec<(u64, u32)>,
    pub last_host: Vec<AtomicU32>,
}

impl ParamLinks {
    pub fn new(mut entries: Vec<(u64, u32)>) -> Self {
        entries.sort_unstable();
        let last_host = entries.iter().map(|_| AtomicU32::new(u32::MAX)).collect();
        Self { entries, last_host }
    }

    pub fn index_of_address(&self, address: u64) -> Option<usize> {
        self.entries.binary_search_by_key(&address, |e| e.0).ok()
    }
}

/// Transport snapshot read by the host-context blocks during render.
#[derive(Clone, Copy, Debug, Default)]
struct HostContext {
    bpm: f64,
    numerator: f64,
    denominator: isize,
    position: f64,
    samples_to_next_beat: isize,
    bar_start: f64,
    sample_position: f64,
    playing: bool,
    recording: bool,
    cycling: bool,
    loop_start: f64,
    loop_end: f64,
    changed: bool,
}

/// Audio-thread state reachable from the blocks (through a raw pointer).
pub(crate) struct RtState {
    in_bufs: Vec<Vec<f32>>,
    frames: u32,
    ctx: HostContext,
    block_start: f64,
    midi_out: Vec<(u32, [u8; 3])>,
}

/// Raw pointer to the node's [`RtState`], captured by the blocks.
#[derive(Clone, Copy)]
struct RtPtr(*mut RtState);

impl RtPtr {
    /// SAFETY: only called from blocks invoked synchronously inside the node's render call
    /// (audio thread) while the node, which owns the state, is alive.
    unsafe fn get(self) -> &'static mut RtState {
        unsafe { &mut *self.0 }
    }
}

/// Blocks handed to the unit (host context, MIDI out); kept alive by the node.
pub(crate) struct HostBlocks {
    pub musical: RcBlock<MusicalContextFn>,
    pub transport: RcBlock<TransportStateFn>,
    pub midi_out: RcBlock<MidiOutFn>,
}

struct ParamValue {
    id: u32,
    address: u64,
    link: usize,
    value: f64,
}

pub(crate) struct AuNode {
    // Blocks and the unit first: they drop before the state they point to.
    render: RcBlock<RenderFn>,
    pull: RcBlock<PullFn>,
    schedule_param: Option<RcBlock<ScheduleParamFn>>,
    schedule_midi: Option<RcBlock<ScheduleMidiFn>>,
    _host_blocks: HostBlocks,
    au: Retained<AUAudioUnit>,
    rt: Box<UnsafeCell<RtState>>,
    out_bufs: Vec<Vec<f32>>,
    out_list: Box<BufList>,
    values: Vec<ParamValue>,
    pending: Vec<(u32, f64)>,
    links: Arc<ParamLinks>,
    shared: Arc<NodeShared>,
    descriptor: DeviceDescriptor,
    has_input: bool,
    in_channels: usize,
    out_channels: usize,
    max_frames: usize,
    sample_rate: f64,
    sample_time: f64,
    errors: u32,
    reset_imp: Option<ResetImp>,
}

type ResetImp = unsafe extern "C-unwind" fn(*mut AnyObject, Sel);

/// The implementation of `-reset` for `au`'s class.
fn reset_imp(au: &AUAudioUnit) -> Option<ResetImp> {
    let method = au.class().instance_method(sel!(reset))?;
    // SAFETY: `reset` has the signature `-(void)reset`, i.e. `fn(self, _cmd)`.
    Some(unsafe { std::mem::transmute::<Imp, ResetImp>(method.implementation()) })
}

// SAFETY: the node is created on the main thread and then used by exactly one thread at a
// time (audio thread, then the GC/main thread for drop). The unit's render/schedule blocks
// are documented as callable from the render thread; ObjC retain/release are thread-safe.
unsafe impl Send for AuNode {}

pub(crate) struct NodeInit {
    pub au: Retained<AUAudioUnit>,
    pub render: RcBlock<RenderFn>,
    pub schedule_param: Option<RcBlock<ScheduleParamFn>>,
    pub schedule_midi: Option<RcBlock<ScheduleMidiFn>>,
    pub links: Arc<ParamLinks>,
    pub shared: Arc<NodeShared>,
    pub descriptor: DeviceDescriptor,
    pub has_input: bool,
    pub in_channels: usize,
    pub out_channels: usize,
    pub max_frames: usize,
    pub sample_rate: f64,
    /// `(id, address, value)`.
    pub values: Vec<(u32, u64, f64)>,
}

/// State + the blocks reading it. Created before the node so the host blocks can be set on
/// the unit before `allocateRenderResources` (a requirement for the context blocks).
pub(crate) struct RtParts {
    rt: Box<UnsafeCell<RtState>>,
    pull: RcBlock<PullFn>,
    pub host: HostBlocks,
}

pub(crate) fn rt_parts(in_channels: usize, max_frames: usize) -> RtParts {
    let rt = Box::new(UnsafeCell::new(RtState {
        in_bufs: (0..in_channels).map(|_| vec![0.0; max_frames]).collect(),
        frames: 0,
        ctx: HostContext::default(),
        block_start: 0.0,
        midi_out: Vec::with_capacity(MIDI_OUT_CAP),
    }));
    let ptr = RtPtr(rt.get());

    let pull = RcBlock::new(
        move |_flags: NonNull<AudioUnitRenderActionFlags>,
              _ts: NonNull<AudioTimeStamp>,
              frames: u32,
              _bus: isize,
              data: NonNull<AudioBufferList>|
              -> i32 {
            // SAFETY: called synchronously from inside our render call.
            let st = unsafe { ptr.get() };
            let list = data.as_ptr() as *mut BufList;
            // SAFETY: the unit passes a valid list with `count` buffers.
            let count = unsafe { (*list).count } as usize;
            let frames = (frames.min(st.frames)) as usize;
            for c in 0..count.min(MAX_CHANNELS) {
                // SAFETY: `c < count`; buffers are laid out contiguously.
                let buf = unsafe { &mut *(*list).buffers.as_mut_ptr().add(c) };
                let src = st.in_bufs.get(c).or(st.in_bufs.last());
                let bytes = (frames * 4) as u32;
                match src {
                    Some(src) if buf.mData.is_null() => {
                        buf.mData = src.as_ptr() as *mut _;
                        buf.mDataByteSize = bytes;
                    }
                    Some(src) => {
                        let n = frames.min(buf.mDataByteSize as usize / 4);
                        // SAFETY: the unit's buffer holds `mDataByteSize` bytes.
                        unsafe {
                            std::ptr::copy_nonoverlapping(src.as_ptr(), buf.mData as *mut f32, n)
                        };
                        buf.mDataByteSize = (n * 4) as u32;
                    }
                    None if !buf.mData.is_null() => {
                        // SAFETY: as above.
                        unsafe {
                            std::ptr::write_bytes(
                                buf.mData as *mut u8,
                                0,
                                buf.mDataByteSize as usize,
                            )
                        };
                    }
                    None => return -10863, // kAudioUnitErr_CannotDoInCurrentContext
                }
            }
            0
        },
    );

    let musical = RcBlock::new(
        move |tempo: *mut f64,
              num: *mut f64,
              den: *mut isize,
              beat: *mut f64,
              to_next: *mut isize,
              downbeat: *mut f64|
              -> Bool {
            // SAFETY: see RtPtr::get; out pointers may be null.
            let c = unsafe { ptr.get() }.ctx;
            unsafe {
                if let Some(p) = tempo.as_mut() {
                    *p = c.bpm;
                }
                if let Some(p) = num.as_mut() {
                    *p = c.numerator;
                }
                if let Some(p) = den.as_mut() {
                    *p = c.denominator;
                }
                if let Some(p) = beat.as_mut() {
                    *p = c.position;
                }
                if let Some(p) = to_next.as_mut() {
                    *p = c.samples_to_next_beat;
                }
                if let Some(p) = downbeat.as_mut() {
                    *p = c.bar_start;
                }
            }
            Bool::YES
        },
    );
    let transport = RcBlock::new(
        move |flags: *mut AUHostTransportStateFlags,
              sample_pos: *mut f64,
              cycle_start: *mut f64,
              cycle_end: *mut f64|
              -> Bool {
            // SAFETY: see RtPtr::get; out pointers may be null.
            let c = unsafe { ptr.get() }.ctx;
            let mut f = AUHostTransportStateFlags(0);
            if c.changed {
                f |= AUHostTransportStateFlags::Changed;
            }
            if c.playing {
                f |= AUHostTransportStateFlags::Moving;
            }
            if c.recording {
                f |= AUHostTransportStateFlags::Recording;
            }
            if c.cycling {
                f |= AUHostTransportStateFlags::Cycling;
            }
            unsafe {
                if let Some(p) = flags.as_mut() {
                    *p = f;
                }
                if let Some(p) = sample_pos.as_mut() {
                    *p = c.sample_position;
                }
                if let Some(p) = cycle_start.as_mut() {
                    *p = c.loop_start;
                }
                if let Some(p) = cycle_end.as_mut() {
                    *p = c.loop_end;
                }
            }
            Bool::YES
        },
    );
    let midi_out = RcBlock::new(
        move |time: i64, _cable: u8, len: isize, bytes: NonNull<u8>| -> i32 {
            // SAFETY: see RtPtr::get; `bytes` holds `len` bytes.
            let st = unsafe { ptr.get() };
            let data = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), len.max(0) as usize) };
            let offset = if time >= 0 {
                (time as f64 - st.block_start).max(0.0) as u32
            } else {
                // Immediate (AUEventSampleTimeImmediate + offset).
                time.wrapping_sub(AUEventSampleTimeImmediate).max(0) as u32
            };
            let mut i = 0;
            while i < data.len() {
                let status = data[i];
                let n = match status & 0xF0 {
                    0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 3,
                    0xC0 | 0xD0 => 2,
                    _ => break, // sysex / system messages: not forwarded
                };
                if i + n > data.len() || st.midi_out.len() == st.midi_out.capacity() {
                    break;
                }
                let mut msg = [0u8; 3];
                msg[..n].copy_from_slice(&data[i..i + n]);
                st.midi_out.push((offset, msg));
                i += n;
            }
            0
        },
    );
    RtParts {
        rt,
        pull,
        host: HostBlocks {
            musical,
            transport,
            midi_out,
        },
    }
}

/// Engine note/MIDI event → MIDI 1.0 bytes (+ length).
fn midi_bytes(kind: &EventKind) -> Option<([u8; 3], usize)> {
    let vel = |v: f32| ((v.clamp(0.0, 1.0) * 127.0).round() as u8).min(127);
    match *kind {
        EventKind::NoteOn {
            channel,
            key,
            velocity,
            ..
        } => Some((
            [0x90 | (channel & 0x0F), key.min(127), vel(velocity).max(1)],
            3,
        )),
        EventKind::NoteOff {
            channel,
            key,
            velocity,
            ..
        } => Some(([0x80 | (channel & 0x0F), key.min(127), vel(velocity)], 3)),
        EventKind::NoteChoke { channel, key, .. } => {
            Some(([0x80 | (channel & 0x0F), key.min(127), 0], 3))
        }
        EventKind::Midi { data } => {
            let n = match data[0] & 0xF0 {
                0xC0 | 0xD0 => 2,
                0x80..=0xE0 => 3,
                _ => return None,
            };
            Some((data, n))
        }
        _ => None,
    }
}

impl AuNode {
    pub(crate) fn new(init: NodeInit, parts: RtParts) -> Self {
        let NodeInit {
            au,
            render,
            schedule_param,
            schedule_midi,
            links,
            shared,
            descriptor,
            has_input,
            in_channels,
            out_channels,
            max_frames,
            sample_rate,
            values,
        } = init;
        let mut values: Vec<ParamValue> = values
            .into_iter()
            .filter_map(|(id, address, value)| {
                Some(ParamValue {
                    id,
                    address,
                    link: links.index_of_address(address)?,
                    value,
                })
            })
            .collect();
        values.sort_by_key(|v| v.id);
        let pending_cap = values.len().max(16);
        let reset_imp = reset_imp(&au);
        Self {
            render,
            pull: parts.pull,
            schedule_param,
            schedule_midi,
            _host_blocks: parts.host,
            au,
            rt: parts.rt,
            out_bufs: (0..out_channels).map(|_| vec![0.0; max_frames]).collect(),
            out_list: BufList::new(),
            values,
            pending: Vec::with_capacity(pending_cap),
            links,
            shared,
            descriptor,
            has_input,
            in_channels,
            out_channels,
            max_frames,
            sample_rate,
            sample_time: 0.0,
            errors: 0,
            reset_imp,
        }
    }

    fn value_index(&self, id: u32) -> Option<usize> {
        self.values.binary_search_by_key(&id, |v| v.id).ok()
    }

    fn schedule(&mut self, offset: u32, id: u32, value: f64) {
        let Some(i) = self.value_index(id) else {
            return;
        };
        let v = &mut self.values[i];
        v.value = value;
        let value = value as f32;
        self.links.last_host[v.link].store(value.to_bits(), Ordering::Relaxed);
        if let Some(block) = &self.schedule_param {
            block.call((
                AUEventSampleTimeImmediate + i64::from(offset),
                0,
                v.address,
                value,
            ));
        }
    }

    fn send_midi(&self, offset: u32, bytes: &[u8; 3], len: usize) {
        if let Some(block) = &self.schedule_midi {
            let ptr = NonNull::from(&bytes[0]);
            block.call((
                AUEventSampleTimeImmediate + i64::from(offset),
                0,
                len as isize,
                ptr,
            ));
        }
    }

    fn update_context(&mut self, t: &TransportInfo) {
        let st = self.rt.get_mut();
        let sig = t.time_signature;
        let prev = st.ctx;
        let next_beat = t.position.ceil();
        let to_next = if t.beats_per_sample > 0.0 {
            ((next_beat - t.position) / t.beats_per_sample).round() as isize
        } else {
            0
        };
        st.ctx = HostContext {
            bpm: t.bpm,
            numerator: f64::from(sig.numerator.max(1)),
            denominator: isize::from(sig.denominator.max(1)),
            position: t.position,
            samples_to_next_beat: to_next,
            bar_start: t.bar_start,
            sample_position: t.seconds * self.sample_rate,
            playing: t.playing,
            recording: t.recording,
            cycling: t.loop_active,
            loop_start: t.loop_start,
            loop_end: t.loop_end,
            changed: false,
        };
        st.ctx.changed = prev.playing != t.playing
            || prev.recording != t.recording
            || prev.cycling != t.loop_active;
    }

    fn fault(&mut self, audio: &mut AudioBuffers<'_, '_>) -> ProcessStatus {
        self.shared.faulted.store(true, Ordering::Release);
        audio.clear_outputs();
        ProcessStatus::Silent
    }
}

impl Node for AuNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        if config.max_block_size > self.max_frames {
            tracing::warn!(
                "AU node prepared for {} frames but activated for {}",
                config.max_block_size,
                self.max_frames
            );
        }
    }

    fn reset(&mut self) {
        // `-[AUAudioUnit reset]` "may be invoked on a render thread": clears delay lines,
        // voices. Called through its cached IMP (no message dispatch / encoding checks).
        if let Some(imp) = self.reset_imp {
            let obj = Retained::as_ptr(&self.au) as *mut AnyObject;
            // SAFETY: `imp` is the implementation of `reset` for the unit's class (looked up
            // at creation); `reset` takes no arguments and returns void.
            unsafe { imp(obj, sel!(reset)) };
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let frames = ctx.frames;
        if self.shared.faulted.load(Ordering::Relaxed) || frames > self.max_frames {
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        // Render events: pending immediate sets, then the engine's (sorted) events.
        for i in 0..self.pending.len() {
            let (id, value) = self.pending[i];
            self.schedule(0, id, value);
        }
        self.pending.clear();
        for e in ctx.events {
            match &e.kind {
                EventKind::Param { param, value } => self.schedule(e.offset, param.0, *value),
                EventKind::AllNotesOff => {
                    for ch in 0..16u8 {
                        self.send_midi(e.offset, &[0xB0 | ch, 123, 0], 3);
                        self.send_midi(e.offset, &[0xB0 | ch, 64, 0], 3);
                    }
                }
                kind => {
                    if let Some((bytes, len)) = midi_bytes(kind) {
                        self.send_midi(e.offset, &bytes, len);
                    }
                }
            }
        }
        self.update_context(ctx.transport);

        // Input for the pull block.
        {
            let st = self.rt.get_mut();
            st.frames = frames as u32;
            st.block_start = self.sample_time;
            st.midi_out.clear();
            for (c, dst) in st.in_bufs.iter_mut().enumerate() {
                match audio.inputs.get(c).or(audio.inputs.last()) {
                    Some(src) => dst[..frames].copy_from_slice(&src[..frames]),
                    None => dst[..frames].fill(0.0),
                }
            }
        }

        // Output buffers: ours (the unit may substitute its own pointers).
        let list = &mut *self.out_list;
        list.count = self.out_channels as u32;
        for (c, buf) in self.out_bufs.iter_mut().enumerate() {
            list.buffers[c] = AudioBuffer {
                mNumberChannels: 1,
                mDataByteSize: (frames * 4) as u32,
                mData: buf.as_mut_ptr() as *mut _,
            };
        }
        let mut flags = AudioUnitRenderActionFlags(0);
        // SAFETY: AudioTimeStamp is plain old data; all-zero is valid.
        let mut ts: AudioTimeStamp = unsafe { std::mem::zeroed() };
        ts.mSampleTime = self.sample_time;
        ts.mFlags = AudioTimeStampFlags::SampleTimeValid;
        let pull = if self.has_input && self.in_channels > 0 {
            RcBlock::as_ptr(&self.pull)
        } else {
            std::ptr::null_mut()
        };
        let status = self.render.call((
            NonNull::from(&mut flags),
            NonNull::from(&mut ts),
            frames as u32,
            0,
            NonNull::from(&mut *self.out_list).cast(),
            pull,
        ));
        self.sample_time += frames as f64;
        if status != 0 {
            self.errors += 1;
            if self.errors >= MAX_RENDER_ERRORS {
                return self.fault(audio);
            }
            audio.clear_outputs();
            return ProcessStatus::Continue;
        }
        self.errors = 0;

        let list = &*self.out_list;
        let n = (list.count as usize).min(self.out_channels);
        if n == 0 {
            audio.clear_outputs();
        } else {
            for (c, dst) in audio.outputs.iter_mut().enumerate() {
                let b = &list.buffers[c.min(n - 1)];
                let avail = (b.mDataByteSize as usize / 4).min(frames);
                if b.mData.is_null() {
                    dst[..frames].fill(0.0);
                    continue;
                }
                // SAFETY: the unit filled `avail` samples at `mData` (ours or its own).
                let src = unsafe { std::slice::from_raw_parts(b.mData as *const f32, avail) };
                dst[..avail].copy_from_slice(src);
                dst[avail..frames].fill(0.0);
            }
        }

        // MIDI output (note effects / MIDI processors).
        let last = (frames - 1) as u32;
        let st = self.rt.get_mut();
        for &(offset, data) in &st.midi_out {
            let offset = offset.min(last);
            let [status, d1, d2] = data;
            let kind = match status & 0xF0 {
                0x90 if d2 > 0 => EventKind::NoteOn {
                    note_id: u32::from(d1),
                    channel: status & 0x0F,
                    key: d1,
                    velocity: f32::from(d2) / 127.0,
                },
                0x80 | 0x90 => EventKind::NoteOff {
                    note_id: u32::from(d1),
                    channel: status & 0x0F,
                    key: d1,
                    velocity: f32::from(d2) / 127.0,
                },
                _ => EventKind::Midi { data },
            };
            ctx.out_events.push(ProcessEvent { offset, kind });
        }
        ProcessStatus::Continue
    }

    fn latency(&self) -> u32 {
        self.shared.latency.load(Ordering::Relaxed)
    }

    fn channels(&self) -> (u16, u16) {
        (
            if self.has_input {
                self.in_channels as u16
            } else {
                0
            },
            self.out_channels as u16,
        )
    }
}

impl Device for AuNode {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.value_index(id.0).map(|i| self.values[i].value)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        if let Some(i) = self.value_index(id.0) {
            self.values[i].value = value;
        }
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((id.0, value));
        }
    }
}

impl PluginNode for AuNode {
    fn is_faulted(&self) -> bool {
        self.shared.faulted.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_events_to_midi() {
        let on = EventKind::NoteOn {
            note_id: 1,
            channel: 2,
            key: 60,
            velocity: 1.0,
        };
        assert_eq!(midi_bytes(&on), Some(([0x92, 60, 127], 3)));
        let quiet = EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 0.0,
        };
        assert_eq!(
            midi_bytes(&quiet).unwrap().0[2],
            1,
            "velocity 0 would be a note-off"
        );
        let off = EventKind::NoteOff {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 0.5,
        };
        assert_eq!(midi_bytes(&off), Some(([0x80, 60, 64], 3)));
        assert_eq!(
            midi_bytes(&EventKind::Midi { data: [0xC1, 5, 0] }),
            Some(([0xC1, 5, 0], 2))
        );
        assert_eq!(midi_bytes(&EventKind::Midi { data: [0xF8, 0, 0] }), None);
        assert_eq!(midi_bytes(&EventKind::AllNotesOff), None);
    }
}
