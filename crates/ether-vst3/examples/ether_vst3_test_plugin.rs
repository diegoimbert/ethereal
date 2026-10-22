//! Minimal VST3 plugin used by the `ether-vst3`, `ether-plugin-scanner` and `ether-sandbox`
//! tests, written with the `vst3` crate's plugin side (`ComWrapper` classes). Wrapped into a
//! `.vst3` bundle by `ether_vst3::testing::make_bundle`.
//!
//! Three audio-module classes:
//!
//! **Gain effect** (`E7E1E4A1000000000000000000000001`, `Fx|Dynamics`): a processor with a
//! *separate* edit controller, connected through `IConnectionPoint`. Stereo in/out.
//! - param 1 `Gain`: continuous, output gain = 2 × normalized (default 0.5 = unity), applied
//!   sample-accurately from `IParameterChanges` points; text = the gain (`"1.00"`), units
//!   `x`;
//! - param 2 `Mode`: 2 steps (Normal / Invert / Mute), sample-accurate;
//! - param 3 `Latency`: 3 steps, latency = step × 64 samples (default 1 = 64). Setting it on
//!   the controller sends an `IMessage` (created through the host's `IHostApplication`) to
//!   the processor and calls `restartComponent(kLatencyChanged)`;
//! - param 4 `Trigger` (hidden, controller-only, kept in the controller state): setting it to
//!   `v` makes the controller emit `beginEdit(1)`, `performEdit(1, v)`, `endEdit(1)` and
//!   `setDirty(true)` through the component handler, as if the user moved the gain knob in
//!   the GUI.
//! - An `IPlugView` editor (300×200, resizable) that accepts any parent. On Linux, like a
//!   real Linux editor, it gets the host's `Linux::IRunLoop` from its `IPlugFrame` when
//!   attached and registers a timer (10 ms) and an event handler on a pipe it makes readable
//!   (one byte per attach), unregistering both when removed; `ether_vst3_test_run_loop`
//!   reports the timer ticks, the handler calls and the registrations alive.
//! - A second, stereo `kAux` input bus `Sidechain` (not active by default; the aux bus of
//!   plugin sidechains, CONTRACTS §12.14): added to the output after the gain, when the
//!   host activated it and passes it. Hosts feed it silence without a sidechain source, so
//!   the output is unchanged then.
//!
//! **Instrument** (`E7E1E4A1000000000000000000000002`, `Instrument|Synth`): a single
//! component (processor and controller in one object). No audio input, stereo output, one
//! event input bus. Outputs a DC level = `Level` × velocity (× -1 if `Invert`) from a note-on
//! to its note-off, sample-accurately.
//! - param 10 `Level`: continuous, default 1;
//! - param 11 `Invert`: 1 step (toggle).
//!
//! Loading aborts the process if the module path contains `ether-crash`, and hangs forever if
//! it contains `ether-hang` (scanner crash/timeout tests).
//!
//! **Tail effect** (`E7E1E4A1000000000000000000000003`, `Fx|Delay`): a single component,
//! stereo in/out, for the host's sleep on silence. What it does is param 20 `Kind`
//! (3 steps):
//! - 0 `Echo`: the input [`ECHO`] samples later (no dry signal), tail = [`ECHO`]; flags its
//!   output silent when it is;
//! - 1 `Generator`: a click (0.5) every [`CLICK_PERIOD`] samples out of nothing, tail =
//!   `kInfiniteTail`;
//! - 2 `Liar`: the same clicks, but tail = `kNoTail` (a misbehaving plugin);
//! - 3 `Flagged`: writes 0.5 everywhere and flags the outputs silent whenever the input
//!   is (hosts must honour the flags).
//!
//! Hidden, read-only params report what the processor saw: 21 `Calls` (process calls with
//! frames, normalized = calls / 10⁶) and 22 `InFlags` (the main input's last silence flags,
//! normalized = flags / 3). Param 23 `Cost` burns `1000 × normalized` dependent
//! multiply-adds per frame (an expensive plugin for benchmarks).
//!
//! A note-on with key 127 makes the instrument's `process` return `kInternalError` (host
//! fault handling).
//!
//! Every COM object of the fixture is counted; the module exit function (`bundleExit` /
//! `ModuleExit` / `ExitDll`) aborts the process if any is still alive when the last module
//! reference goes away, so a host that leaks plugin objects fails its tests.
//!
//! Nothing here allocates on the audio thread.
// SDK enum constants are `u32` or `i32` depending on the OS, hence the casts.
#![allow(
    non_snake_case,
    unsafe_op_in_unsafe_fn,
    clippy::missing_safety_doc,
    clippy::unnecessary_cast
)]

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};

use vst3::Steinberg::Vst::BusDirections_::{kInput, kOutput};
use vst3::Steinberg::Vst::BusInfo_::BusFlags_::kDefaultActive;
use vst3::Steinberg::Vst::BusTypes_::{kAux, kMain};
use vst3::Steinberg::Vst::Event_::EventTypes_::{kNoteOffEvent, kNoteOnEvent};
use vst3::Steinberg::Vst::MediaTypes_::{kAudio, kEvent};
use vst3::Steinberg::Vst::ParameterInfo_::ParameterFlags_::{kCanAutomate, kIsHidden, kIsReadOnly};
use vst3::Steinberg::Vst::RestartFlags_::kLatencyChanged;
use vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComRef, ComWrapper, Interface, uid};

const EFFECT_CID: TUID = uid(0xE7E1E4A1, 0, 0, 1);
const INSTRUMENT_CID: TUID = uid(0xE7E1E4A1, 0, 0, 2);
const EFFECT_CONTROLLER_CID: TUID = uid(0xE7E1E4A1, 0, 0, 0x11);

const GAIN: ParamID = 1;
const MODE: ParamID = 2;
const LATENCY: ParamID = 3;
const TRIGGER: ParamID = 4;
const LEVEL: ParamID = 10;
const INVERT: ParamID = 11;
const KIND: ParamID = 20;
const CALLS: ParamID = 21;
const IN_FLAGS: ParamID = 22;
const COST: ParamID = 23;
const TAIL_CID: TUID = uid(0xE7E1E4A1, 0, 0, 3);

/// Delay of the tail effect's `Echo` kind, in samples.
const ECHO: usize = 300;
/// Samples between the clicks of the `Generator` and `Liar` kinds.
const CLICK_PERIOD: u64 = 1000;

const LATENCY_UNIT: u32 = 64;

// ---------------------------------------------------------------------------------------
// live-object accounting

/// Live COM objects of this module.
static LIVE: AtomicIsize = AtomicIsize::new(0);
/// Balance of entry/exit calls (hosts may load the module several times).
static ENTRIES: AtomicIsize = AtomicIsize::new(0);

/// A member of every fixture class: counts the object in `LIVE`.
struct Live(());

impl Live {
    fn new() -> Self {
        Self::default()
    }
}

impl Default for Live {
    fn default() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Live(())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

fn module_entry() -> bool {
    ENTRIES.fetch_add(1, Ordering::SeqCst);
    true
}

fn module_exit() -> bool {
    if ENTRIES.fetch_sub(1, Ordering::SeqCst) == 1 {
        let live = LIVE.load(Ordering::SeqCst);
        if live != 0 {
            eprintln!("ether_vst3_test_plugin: {live} plugin object(s) leaked at module exit");
            std::process::abort();
        }
    }
    true
}

// ---------------------------------------------------------------------------------------
// helpers

fn copy_cstring(src: &str, dst: &mut [c_char]) {
    let n = src.len().min(dst.len() - 1);
    for (d, s) in dst.iter_mut().zip(src.bytes().take(n)) {
        *d = s as c_char;
    }
    dst[n] = 0;
}

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    let last = dst.len() - 1;
    for (d, s) in dst[..last].iter_mut().zip(src.encode_utf16()) {
        *d = s as TChar;
        n += 1;
    }
    dst[n] = 0;
}

unsafe fn write_bytes(stream: *mut IBStream, bytes: &[u8]) -> bool {
    let Some(s) = ComRef::from_raw(stream) else {
        return false;
    };
    let mut written = 0;
    s.write(
        bytes.as_ptr() as *mut c_void,
        bytes.len() as i32,
        &mut written,
    ) == kResultOk
        && written == bytes.len() as i32
}

unsafe fn read_bytes<const N: usize>(stream: *mut IBStream) -> Option<[u8; N]> {
    let s = ComRef::from_raw(stream)?;
    let mut buf = [0u8; N];
    let mut read = 0;
    (s.read(buf.as_mut_ptr().cast(), N as i32, &mut read) == kResultOk && read == N as i32)
        .then_some(buf)
}

fn load_f64(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Ordering::Relaxed))
}

fn store_f64(a: &AtomicU64, v: f64) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

fn param_info(
    info: &mut ParameterInfo,
    id: ParamID,
    title: &str,
    steps: i32,
    default: f64,
    flags: u32,
) {
    info.id = id;
    copy_wstring(title, &mut info.title);
    copy_wstring(title, &mut info.shortTitle);
    copy_wstring(if id == GAIN { "x" } else { "" }, &mut info.units);
    info.stepCount = steps;
    info.defaultNormalizedValue = default;
    info.unitId = 0;
    info.flags = flags as i32;
}

/// Up to 64 points of one param queue, read without allocating.
struct Points {
    data: [(i32, f64); 64],
    len: usize,
}

impl Points {
    unsafe fn read(queue: &ComRef<'_, IParamValueQueue>) -> Self {
        let mut p = Points {
            data: [(0, 0.0); 64],
            len: 0,
        };
        let count = queue.getPointCount().clamp(0, 64);
        for i in 0..count {
            let (mut o, mut v) = (0, 0.0);
            if queue.getPoint(i, &mut o, &mut v) == kResultOk {
                p.data[p.len] = (o, v);
                p.len += 1;
            }
        }
        p
    }

    fn points(&self) -> &[(i32, f64)] {
        &self.data[..self.len]
    }
}

/// Stereo main bus info.
unsafe fn stereo_bus(bus: *mut BusInfo, dir: BusDirection, name: &str) -> tresult {
    let bus = &mut *bus;
    bus.mediaType = kAudio as MediaType;
    bus.direction = dir;
    bus.channelCount = 2;
    copy_wstring(name, &mut bus.name);
    bus.busType = kMain as BusType;
    bus.flags = kDefaultActive as u32;
    kResultOk
}

unsafe fn channel_slices<'a>(
    buses: *mut AudioBusBuffers,
    count: i32,
    frames: usize,
) -> Option<(&'a mut [f32], &'a mut [f32])> {
    bus_slices(buses, count, 0, frames)
}

/// Left/right channels of bus `index` (at least stereo) of `buses`.
unsafe fn bus_slices<'a>(
    buses: *mut AudioBusBuffers,
    count: i32,
    index: i32,
    frames: usize,
) -> Option<(&'a mut [f32], &'a mut [f32])> {
    if buses.is_null() || count <= index {
        return None;
    }
    let bus = &*buses.add(index as usize);
    if bus.numChannels < 2 {
        return None;
    }
    let chans = bus.__field0.channelBuffers32;
    if chans.is_null() {
        return None;
    }
    let l = std::slice::from_raw_parts_mut(*chans, frames);
    let r = std::slice::from_raw_parts_mut(*chans.add(1), frames);
    Some((l, r))
}

// ---------------------------------------------------------------------------------------
// gain effect: processor

struct EffectProcessor {
    _live: Live,
    gain: AtomicU64,
    mode: AtomicU32,
    latency_step: AtomicU32,
    /// The host activated the `Sidechain` aux bus (`activateBus`).
    aux_active: AtomicBool,
    peer: Mutex<Option<ComPtr<IConnectionPoint>>>,
}

impl Class for EffectProcessor {
    type Interfaces = (IComponent, IAudioProcessor, IConnectionPoint);
}

impl EffectProcessor {
    fn new() -> Self {
        Self {
            _live: Live::new(),
            gain: AtomicU64::new(0.5f64.to_bits()),
            mode: AtomicU32::new(0),
            latency_step: AtomicU32::new(1),
            aux_active: AtomicBool::new(false),
            peer: Mutex::new(None),
        }
    }
}

impl IPluginBaseTrait for EffectProcessor {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for EffectProcessor {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        *class_id = EFFECT_CONTROLLER_CID;
        kResultOk
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> i32 {
        match (media == kAudio as MediaType, dir == kInput as BusDirection) {
            (true, true) => 2,
            (true, false) => 1,
            _ => 0,
        }
    }
    unsafe fn getBusInfo(
        &self,
        media: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if media == kAudio as MediaType && dir == kInput as BusDirection && index == 1 {
            stereo_bus(bus, dir, "Sidechain");
            (*bus).busType = kAux as BusType;
            (*bus).flags = 0;
            return kResultOk;
        }
        if media != kAudio as MediaType || index != 0 {
            return kInvalidArgument;
        }
        let name = if dir == kInput as BusDirection {
            "Input"
        } else {
            "Output"
        };
        stereo_bus(bus, dir, name)
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, m: MediaType, d: BusDirection, i: i32, s: TBool) -> tresult {
        if m == kAudio as MediaType && d == kInput as BusDirection && i == 1 {
            self.aux_active.store(s != 0, Ordering::Relaxed);
        }
        kResultOk
    }
    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let (Some(g), Some(m), Some(l)) = (
            read_bytes::<8>(state),
            read_bytes::<4>(state),
            read_bytes::<4>(state),
        ) else {
            return kResultFalse;
        };
        store_f64(&self.gain, f64::from_le_bytes(g));
        self.mode.store(u32::from_le_bytes(m), Ordering::Relaxed);
        self.latency_step
            .store(u32::from_le_bytes(l), Ordering::Relaxed);
        kResultOk
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let ok = write_bytes(state, &load_f64(&self.gain).to_le_bytes())
            && write_bytes(state, &self.mode.load(Ordering::Relaxed).to_le_bytes())
            && write_bytes(
                state,
                &self.latency_step.load(Ordering::Relaxed).to_le_bytes(),
            );
        if ok { kResultOk } else { kResultFalse }
    }
}

impl IAudioProcessorTrait for EffectProcessor {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        if (1..=2).contains(&num_ins)
            && num_outs == 1
            && *inputs == SpeakerArr::kStereo
            && (num_ins == 1 || *inputs.add(1) == SpeakerArr::kStereo)
            && *outputs == SpeakerArr::kStereo
        {
            kResultTrue
        } else {
            kResultFalse
        }
    }
    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        let buses = if dir == kInput as BusDirection { 2 } else { 1 };
        if !(0..buses).contains(&index) {
            return kInvalidArgument;
        }
        *arr = SpeakerArr::kStereo;
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == kSample32 as i32 {
            kResultOk
        } else {
            kNotImplemented
        }
    }
    unsafe fn getLatencySamples(&self) -> u32 {
        self.latency_step.load(Ordering::Relaxed) * LATENCY_UNIT
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let data = &*data;
        let frames = data.numSamples.max(0) as usize;
        let start_gain = load_f64(&self.gain);
        let start_mode = self.mode.load(Ordering::Relaxed);
        let mut gain_points = Points {
            data: [(0, 0.0); 64],
            len: 0,
        };
        let mut mode_points = Points {
            data: [(0, 0.0); 64],
            len: 0,
        };
        if let Some(changes) = ComRef::from_raw(data.inputParameterChanges) {
            for i in 0..changes.getParameterCount() {
                let Some(q) = ComRef::from_raw(changes.getParameterData(i)) else {
                    continue;
                };
                match q.getParameterId() {
                    GAIN => gain_points = Points::read(&q),
                    MODE => mode_points = Points::read(&q),
                    LATENCY => {
                        let p = Points::read(&q);
                        if let Some((_, v)) = p.points().last() {
                            let step = (v * 3.0).round() as u32;
                            self.latency_step.store(step, Ordering::Relaxed);
                        }
                    }
                    _ => {}
                }
            }
        }
        // Final values (also for zero-sample "flush" calls, which have no sample loop).
        if let Some(&(_, v)) = gain_points.points().last() {
            store_f64(&self.gain, v);
        }
        if let Some(&(_, v)) = mode_points.points().last() {
            self.mode.store((v * 2.0).round() as u32, Ordering::Relaxed);
        }
        let Some((in_l, in_r)) = channel_slices(data.inputs, data.numInputs, frames) else {
            return kResultOk;
        };
        let Some((out_l, out_r)) = channel_slices(data.outputs, data.numOutputs, frames) else {
            return kResultOk;
        };
        let aux = if self.aux_active.load(Ordering::Relaxed) {
            bus_slices(data.inputs, data.numInputs, 1, frames)
        } else {
            None
        };
        // Replay the points sample-accurately, from the values at block start.
        let mut gain = start_gain;
        let mut mode = start_mode;
        let (mut gi, mut mi) = (0, 0);
        for i in 0..frames {
            while let Some(&(o, v)) = gain_points.points().get(gi) {
                if o as usize > i {
                    break;
                }
                gain = v;
                gi += 1;
            }
            while let Some(&(o, v)) = mode_points.points().get(mi) {
                if o as usize > i {
                    break;
                }
                mode = (v * 2.0).round() as u32;
                mi += 1;
            }
            let g = (gain * 2.0) as f32
                * match mode {
                    0 => 1.0,
                    1 => -1.0,
                    _ => 0.0,
                };
            out_l[i] = in_l[i] * g;
            out_r[i] = in_r[i] * g;
            if let Some((sc_l, sc_r)) = &aux {
                out_l[i] += sc_l[i];
                out_r[i] += sc_r[i];
            }
        }
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

impl IConnectionPointTrait for EffectProcessor {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> tresult {
        *self.peer.lock().unwrap() = ComRef::from_raw(other).map(|r| r.to_com_ptr());
        kResultOk
    }
    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        *self.peer.lock().unwrap() = None;
        kResultOk
    }
    unsafe fn notify(&self, message: *mut IMessage) -> tresult {
        let Some(msg) = ComRef::from_raw(message) else {
            return kInvalidArgument;
        };
        let id = msg.getMessageID();
        if id.is_null() || CStr::from_ptr(id) != c"latency" {
            return kResultFalse;
        }
        let Some(attrs) = ComRef::from_raw(msg.getAttributes()) else {
            return kResultFalse;
        };
        let mut step = 0i64;
        if attrs.getInt(c"step".as_ptr(), &mut step) != kResultOk {
            return kResultFalse;
        }
        self.latency_step.store(step as u32, Ordering::Relaxed);
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------
// gain effect: controller + editor

struct EffectController {
    _live: Live,
    values: [Cell<f64>; 4],
    host: Cell<Option<*mut FUnknown>>,
    handler: Cell<Option<*mut IComponentHandler>>,
    peer: Cell<Option<*mut IConnectionPoint>>,
}

impl Class for EffectController {
    type Interfaces = (IEditController, IConnectionPoint);
}

impl EffectController {
    fn new() -> Self {
        Self {
            _live: Live::new(),
            values: [
                Cell::new(0.5),
                Cell::new(0.0),
                Cell::new(1.0 / 3.0),
                Cell::new(0.0),
            ],
            host: Cell::new(None),
            handler: Cell::new(None),
            peer: Cell::new(None),
        }
    }

    fn slot(id: ParamID) -> Option<usize> {
        (GAIN..=TRIGGER).contains(&id).then(|| (id - GAIN) as usize)
    }

    unsafe fn send_latency(&self, step: i64) {
        let (Some(host), Some(peer)) = (self.host.get(), self.peer.get()) else {
            return;
        };
        let Some(host) =
            ComRef::<FUnknown>::from_raw(host).and_then(|h| h.cast::<IHostApplication>())
        else {
            return;
        };
        let mut cid = IMessage::IID.map(|b| b as c_char);
        let mut iid = cid;
        let mut obj = std::ptr::null_mut();
        if host.createInstance(&mut cid, &mut iid, &mut obj) != kResultOk {
            return;
        }
        let Some(msg) = ComPtr::<IMessage>::from_raw(obj.cast()) else {
            return;
        };
        msg.setMessageID(c"latency".as_ptr());
        if let Some(attrs) = ComRef::from_raw(msg.getAttributes()) {
            attrs.setInt(c"step".as_ptr(), step);
        }
        if let Some(peer) = ComRef::from_raw(peer) {
            peer.notify(msg.as_ptr());
        }
    }
}

impl IPluginBaseTrait for EffectController {
    unsafe fn initialize(&self, context: *mut FUnknown) -> tresult {
        // The host context outlives the controller (terminate is called first).
        self.host.set((!context.is_null()).then_some(context));
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        self.host.set(None);
        kResultOk
    }
}

impl IConnectionPointTrait for EffectController {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> tresult {
        self.peer.set((!other.is_null()).then_some(other));
        kResultOk
    }
    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        self.peer.set(None);
        kResultOk
    }
    unsafe fn notify(&self, _message: *mut IMessage) -> tresult {
        kResultFalse
    }
}

impl IEditControllerTrait for EffectController {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let (Some(g), Some(m), Some(l)) = (
            read_bytes::<8>(state),
            read_bytes::<4>(state),
            read_bytes::<4>(state),
        ) else {
            return kResultFalse;
        };
        self.values[0].set(f64::from_le_bytes(g));
        self.values[1].set(f64::from(u32::from_le_bytes(m)) / 2.0);
        self.values[2].set(f64::from(u32::from_le_bytes(l)) / 3.0);
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        match read_bytes::<8>(state) {
            Some(t) => {
                self.values[3].set(f64::from_le_bytes(t));
                kResultOk
            }
            None => kResultFalse,
        }
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        if write_bytes(state, &self.values[3].get().to_le_bytes()) {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getParameterCount(&self) -> i32 {
        4
    }
    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        let info = &mut *info;
        match index {
            0 => param_info(info, GAIN, "Gain", 0, 0.5, kCanAutomate as u32),
            1 => param_info(info, MODE, "Mode", 2, 0.0, kCanAutomate as u32),
            2 => param_info(info, LATENCY, "Latency", 3, 1.0 / 3.0, kCanAutomate as u32),
            3 => param_info(info, TRIGGER, "Trigger", 0, 0.0, kIsHidden as u32),
            _ => return kInvalidArgument,
        }
        kResultOk
    }
    unsafe fn getParamStringByValue(
        &self,
        id: ParamID,
        value: ParamValue,
        string: *mut String128,
    ) -> tresult {
        let s = match id {
            MODE => ["Normal", "Invert", "Mute"][((value * 2.0).round() as usize).min(2)].into(),
            LATENCY => format!("{} smp", (value * 3.0).round() as u32 * LATENCY_UNIT),
            GAIN => format!("{:.2}", value * 2.0),
            _ => format!("{value:.3}"),
        };
        copy_wstring(&s, &mut *string);
        kResultOk
    }
    unsafe fn getParamValueByString(
        &self,
        _id: ParamID,
        _string: *mut TChar,
        _value: *mut ParamValue,
    ) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value
    }
    unsafe fn plainParamToNormalized(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value
    }
    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        Self::slot(id).map_or(0.0, |i| self.values[i].get())
    }
    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        let Some(i) = Self::slot(id) else {
            return kInvalidArgument;
        };
        let old = self.values[i].replace(value);
        let handler = self.handler.get().and_then(|h| ComRef::from_raw(h));
        match id {
            LATENCY if old != value => {
                self.send_latency((value * 3.0).round() as i64);
                if let Some(h) = handler {
                    h.restartComponent(kLatencyChanged as i32);
                }
            }
            TRIGGER => {
                if let Some(h) = handler {
                    h.beginEdit(GAIN);
                    self.values[0].set(value);
                    h.performEdit(GAIN, value);
                    h.endEdit(GAIN);
                    if let Some(h2) = h.cast::<IComponentHandler2>() {
                        h2.setDirty(1);
                    }
                }
            }
            _ => {}
        }
        kResultOk
    }
    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        // Borrowed: the host keeps it alive until it passes null (before terminate).
        self.handler.set((!handler.is_null()).then_some(handler));
        kResultOk
    }
    unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
        if name.is_null() || CStr::from_ptr(name) != c"editor" {
            return std::ptr::null_mut();
        }
        ComWrapper::new(TestView::default())
            .to_com_ptr::<IPlugView>()
            .map_or(std::ptr::null_mut(), |p| p.into_raw())
    }
}

#[derive(Default)]
struct TestView {
    _live: Live,
    size: Cell<(i32, i32)>,
    attached: Cell<bool>,
    frame: Cell<Option<*mut IPlugFrame>>,
    #[cfg(target_os = "linux")]
    run_loop: std::cell::RefCell<Option<linux::Registration>>,
}

/// Linux: the editor's use of the host run loop (see the module docs).
#[cfg(target_os = "linux")]
mod linux {
    use std::io::{PipeReader, PipeWriter, Read, Write};
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicU32, Ordering};

    use vst3::Steinberg::Linux::{
        FileDescriptor, IEventHandler, IEventHandlerTrait, IRunLoop, IRunLoopTrait, ITimerHandler,
        ITimerHandlerTrait,
    };
    use vst3::Steinberg::{IPlugFrame, kResultOk};
    use vst3::{Class, ComPtr, ComRef, ComWrapper};

    use super::Live;

    /// Timer ticks, `onFDIsSet` calls, registrations alive.
    pub static COUNTS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

    struct Client {
        _live: Live,
        reader: PipeReader,
    }

    impl Class for Client {
        type Interfaces = (IEventHandler, ITimerHandler);
    }

    impl ITimerHandlerTrait for Client {
        unsafe fn onTimer(&self) {
            COUNTS[0].fetch_add(1, Ordering::SeqCst);
        }
    }

    impl IEventHandlerTrait for Client {
        unsafe fn onFDIsSet(&self, fd: FileDescriptor) {
            let mut b = [0u8; 1];
            if fd == self.reader.as_raw_fd() && (&self.reader).read(&mut b).ok() == Some(1) {
                COUNTS[1].fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// What `attached` registered, undone by `removed`.
    pub struct Registration {
        run_loop: ComPtr<IRunLoop>,
        client: ComWrapper<Client>,
        _writer: PipeWriter,
    }

    impl Registration {
        /// Register with the frame's run loop (`None`: the host has none).
        pub unsafe fn new(frame: *mut IPlugFrame) -> Option<Self> {
            let run_loop = unsafe { ComRef::from_raw(frame) }?.cast::<IRunLoop>()?;
            let (reader, mut writer) = std::io::pipe().ok()?;
            writer.write_all(&[1]).ok()?;
            let fd = reader.as_raw_fd();
            let client = ComWrapper::new(Client {
                _live: Live::new(),
                reader,
            });
            let timer = client.as_com_ref::<ITimerHandler>()?;
            let events = client.as_com_ref::<IEventHandler>()?;
            unsafe {
                if run_loop.registerTimer(timer.as_ptr(), 10) == kResultOk {
                    COUNTS[2].fetch_add(1, Ordering::SeqCst);
                }
                if run_loop.registerEventHandler(events.as_ptr(), fd) == kResultOk {
                    COUNTS[2].fetch_add(1, Ordering::SeqCst);
                }
            }
            Some(Self {
                run_loop,
                client,
                _writer: writer,
            })
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            unsafe {
                if let Some(t) = self.client.as_com_ref::<ITimerHandler>()
                    && self.run_loop.unregisterTimer(t.as_ptr()) == kResultOk
                {
                    COUNTS[2].fetch_sub(1, Ordering::SeqCst);
                }
                if let Some(e) = self.client.as_com_ref::<IEventHandler>()
                    && self.run_loop.unregisterEventHandler(e.as_ptr()) == kResultOk
                {
                    COUNTS[2].fetch_sub(1, Ordering::SeqCst);
                }
            }
        }
    }
}

/// Test hook (Linux): the editor's run-loop counts (timer ticks, `onFDIsSet` calls,
/// registrations alive) into `out[0..3]`; zeros elsewhere.
///
/// # Safety
/// `out` points to 3 writable `u32`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ether_vst3_test_run_loop(out: *mut u32) {
    for i in 0..3 {
        #[cfg(target_os = "linux")]
        let v = linux::COUNTS[i].load(Ordering::SeqCst);
        #[cfg(not(target_os = "linux"))]
        let v = 0;
        *out.add(i) = v;
    }
}

impl Class for TestView {
    type Interfaces = (IPlugView,);
}

impl IPlugViewTrait for TestView {
    unsafe fn isPlatformTypeSupported(&self, _type: FIDString) -> tresult {
        kResultTrue
    }
    unsafe fn attached(&self, parent: *mut c_void, _type: FIDString) -> tresult {
        if parent.is_null() {
            return kInvalidArgument;
        }
        self.attached.set(true);
        #[cfg(target_os = "linux")]
        if let Some(frame) = self.frame.get() {
            *self.run_loop.borrow_mut() = linux::Registration::new(frame);
        }
        kResultOk
    }
    unsafe fn removed(&self) -> tresult {
        self.attached.set(false);
        #[cfg(target_os = "linux")]
        drop(self.run_loop.borrow_mut().take());
        kResultOk
    }
    unsafe fn onWheel(&self, _d: f32) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyDown(&self, _k: char16, _c: i16, _m: i16) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyUp(&self, _k: char16, _c: i16, _m: i16) -> tresult {
        kResultFalse
    }
    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        let (w, h) = match self.size.get() {
            (0, 0) => (300, 200),
            s => s,
        };
        *size = ViewRect {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        kResultOk
    }
    unsafe fn onSize(&self, size: *mut ViewRect) -> tresult {
        let r = &*size;
        self.size.set((r.right - r.left, r.bottom - r.top));
        kResultOk
    }
    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        // Borrowed, as the SDK says: valid until the host passes null.
        self.frame.set((!frame.is_null()).then_some(frame));
        kResultOk
    }
    unsafe fn canResize(&self) -> tresult {
        kResultTrue
    }
    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        let r = &mut *rect;
        r.right = r.left + (r.right - r.left).max(100);
        r.bottom = r.top + (r.bottom - r.top).max(100);
        kResultOk
    }
}

// ---------------------------------------------------------------------------------------
// instrument (single component)

struct Instrument {
    _live: Live,
    level: AtomicU64,
    invert: AtomicBool,
    /// Velocity of the sounding note (0 = silent).
    velocity: AtomicU64,
    handler: Cell<Option<*mut IComponentHandler>>,
}

impl Class for Instrument {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}

impl Instrument {
    fn new() -> Self {
        Self {
            _live: Live::new(),
            level: AtomicU64::new(1.0f64.to_bits()),
            invert: AtomicBool::new(false),
            velocity: AtomicU64::new(0.0f64.to_bits()),
            handler: Cell::new(None),
        }
    }
}

impl IPluginBaseTrait for Instrument {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for Instrument {
    unsafe fn getControllerClassId(&self, _class_id: *mut TUID) -> tresult {
        kNotImplemented
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> i32 {
        let input = dir == kInput as BusDirection;
        match media {
            m if m == kAudio as MediaType && !input => 1,
            m if m == kEvent as MediaType && input => 1,
            _ => 0,
        }
    }
    unsafe fn getBusInfo(
        &self,
        media: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        if media == kAudio as MediaType && dir == kOutput as BusDirection {
            return stereo_bus(bus, dir, "Output");
        }
        if media == kEvent as MediaType && dir == kInput as BusDirection {
            let bus = &mut *bus;
            bus.mediaType = kEvent as MediaType;
            bus.direction = dir;
            bus.channelCount = 16;
            copy_wstring("Notes", &mut bus.name);
            bus.busType = kMain as BusType;
            bus.flags = kDefaultActive as u32;
            return kResultOk;
        }
        kInvalidArgument
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _m: MediaType, _d: BusDirection, _i: i32, _s: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, _state: TBool) -> tresult {
        store_f64(&self.velocity, 0.0);
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let (Some(l), Some(i)) = (read_bytes::<8>(state), read_bytes::<1>(state)) else {
            return kResultFalse;
        };
        store_f64(&self.level, f64::from_le_bytes(l));
        self.invert.store(i[0] != 0, Ordering::Relaxed);
        kResultOk
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let ok = write_bytes(state, &load_f64(&self.level).to_le_bytes())
            && write_bytes(state, &[u8::from(self.invert.load(Ordering::Relaxed))]);
        if ok { kResultOk } else { kResultFalse }
    }
}

impl IAudioProcessorTrait for Instrument {
    unsafe fn setBusArrangements(
        &self,
        _inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        if num_ins == 0 && num_outs == 1 && *outputs == SpeakerArr::kStereo {
            kResultTrue
        } else {
            kResultFalse
        }
    }
    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if dir != kOutput as BusDirection || index != 0 {
            return kInvalidArgument;
        }
        *arr = SpeakerArr::kStereo;
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == kSample32 as i32 {
            kResultOk
        } else {
            kNotImplemented
        }
    }
    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let data = &*data;
        let frames = data.numSamples.max(0) as usize;
        if let Some(changes) = ComRef::from_raw(data.inputParameterChanges) {
            for i in 0..changes.getParameterCount() {
                let Some(q) = ComRef::from_raw(changes.getParameterData(i)) else {
                    continue;
                };
                let p = Points::read(&q);
                let Some(&(_, v)) = p.points().last() else {
                    continue;
                };
                match q.getParameterId() {
                    LEVEL => store_f64(&self.level, v),
                    INVERT => self.invert.store(v >= 0.5, Ordering::Relaxed),
                    _ => {}
                }
            }
        }
        let Some((out_l, out_r)) = channel_slices(data.outputs, data.numOutputs, frames) else {
            return kResultOk;
        };
        let events = ComRef::from_raw(data.inputEvents);
        let count = events.map_or(0, |e| e.getEventCount());
        let level = load_f64(&self.level)
            * if self.invert.load(Ordering::Relaxed) {
                -1.0
            } else {
                1.0
            };
        let mut velocity = load_f64(&self.velocity);
        let mut next = 0;
        for i in 0..frames {
            while next < count {
                let mut e: Event = std::mem::zeroed();
                if events.is_some_and(|l| l.getEvent(next, &mut e) == kResultOk) {
                    if e.sampleOffset as usize > i {
                        break;
                    }
                    if u32::from(e.r#type) == kNoteOnEvent as u32 {
                        if e.__field0.noteOn.pitch == 127 {
                            return kInternalError;
                        }
                        velocity = f64::from(e.__field0.noteOn.velocity);
                    } else if u32::from(e.r#type) == kNoteOffEvent as u32 {
                        velocity = 0.0;
                    }
                }
                next += 1;
            }
            let v = (level * velocity) as f32;
            out_l[i] = v;
            out_r[i] = v;
        }
        store_f64(&self.velocity, velocity);
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

impl IEditControllerTrait for Instrument {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        // Single component: the controller reads the same atomics.
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> i32 {
        2
    }
    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        let info = &mut *info;
        match index {
            0 => param_info(info, LEVEL, "Level", 0, 1.0, kCanAutomate as u32),
            1 => param_info(info, INVERT, "Invert", 1, 0.0, kCanAutomate as u32),
            _ => return kInvalidArgument,
        }
        kResultOk
    }
    unsafe fn getParamStringByValue(
        &self,
        _id: ParamID,
        value: ParamValue,
        string: *mut String128,
    ) -> tresult {
        copy_wstring(&format!("{value:.2}"), &mut *string);
        kResultOk
    }
    unsafe fn getParamValueByString(
        &self,
        _id: ParamID,
        _string: *mut TChar,
        _value: *mut ParamValue,
    ) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value
    }
    unsafe fn plainParamToNormalized(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value
    }
    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        match id {
            LEVEL => load_f64(&self.level),
            INVERT => f64::from(u8::from(self.invert.load(Ordering::Relaxed))),
            _ => 0.0,
        }
    }
    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        // Single component: the controller value is the processor value.
        match id {
            LEVEL => store_f64(&self.level, value),
            INVERT => self.invert.store(value >= 0.5, Ordering::Relaxed),
            _ => return kInvalidArgument,
        }
        kResultOk
    }
    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        self.handler.set((!handler.is_null()).then_some(handler));
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        std::ptr::null_mut()
    }
}

// ---------------------------------------------------------------------------------------
// tail effect (single component)

struct TailDsp {
    ring: [[f32; ECHO]; 2],
    pos: usize,
    /// Samples rendered (the generators' clock).
    clock: u64,
    /// The `Cost` loop's state (kept so the work isn't optimized away).
    burn: f32,
}

struct TailFx {
    _live: Live,
    kind: AtomicU32,
    calls: AtomicU64,
    in_flags: AtomicU64,
    cost: AtomicU64,
    dsp: Mutex<TailDsp>,
}

impl Class for TailFx {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}

impl TailFx {
    fn new() -> Self {
        Self {
            _live: Live::new(),
            kind: AtomicU32::new(0),
            calls: AtomicU64::new(0),
            in_flags: AtomicU64::new(0),
            cost: AtomicU64::new(0.0f64.to_bits()),
            dsp: Mutex::new(TailDsp {
                ring: [[0.0; ECHO]; 2],
                pos: 0,
                clock: 0,
                burn: 0.0,
            }),
        }
    }

    fn set(&self, id: ParamID, v: f64) -> bool {
        match id {
            KIND => self.kind.store((v * 3.0).round() as u32, Ordering::Relaxed),
            COST => store_f64(&self.cost, v),
            _ => return false,
        }
        true
    }
}

impl IPluginBaseTrait for TailFx {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for TailFx {
    unsafe fn getControllerClassId(&self, _class_id: *mut TUID) -> tresult {
        kNotImplemented
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, _dir: BusDirection) -> i32 {
        i32::from(media == kAudio as MediaType)
    }
    unsafe fn getBusInfo(
        &self,
        media: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if index != 0 || media != kAudio as MediaType {
            return kInvalidArgument;
        }
        let name = if dir == kInput as BusDirection {
            "Input"
        } else {
            "Output"
        };
        stereo_bus(bus, dir, name)
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _m: MediaType, _d: BusDirection, _i: i32, _s: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
}

impl IAudioProcessorTrait for TailFx {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        if num_ins == 1
            && num_outs == 1
            && *inputs == SpeakerArr::kStereo
            && *outputs == SpeakerArr::kStereo
        {
            kResultTrue
        } else {
            kResultFalse
        }
    }
    unsafe fn getBusArrangement(
        &self,
        _dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        *arr = SpeakerArr::kStereo;
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == kSample32 as i32 {
            kResultOk
        } else {
            kNotImplemented
        }
    }
    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let data = &mut *data;
        let frames = data.numSamples.max(0) as usize;
        if let Some(changes) = ComRef::from_raw(data.inputParameterChanges) {
            for i in 0..changes.getParameterCount() {
                let Some(q) = ComRef::from_raw(changes.getParameterData(i)) else {
                    continue;
                };
                if let Some(&(_, v)) = Points::read(&q).points().last() {
                    self.set(q.getParameterId(), v);
                }
            }
        }
        if frames == 0 {
            return kResultOk;
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        let in_flags = if data.numInputs > 0 && !data.inputs.is_null() {
            (*data.inputs).silenceFlags
        } else {
            0
        };
        self.in_flags.store(in_flags, Ordering::Relaxed);
        let Some((in_l, in_r)) = channel_slices(data.inputs, data.numInputs, frames) else {
            return kResultOk;
        };
        let Some((out_l, out_r)) = channel_slices(data.outputs, data.numOutputs, frames) else {
            return kResultOk;
        };
        let mut dsp = self.dsp.lock().unwrap();
        let iters = (load_f64(&self.cost) * 1000.0) as usize;
        let mut burn = dsp.burn;
        for _ in 0..frames * iters {
            burn = std::hint::black_box(burn * 0.999_99 + 1e-7);
        }
        dsp.burn = burn;
        let kind = self.kind.load(Ordering::Relaxed);
        let mut silent = true;
        for i in 0..frames {
            let (l, r) = match kind {
                0 => {
                    let pos = dsp.pos;
                    let out = (dsp.ring[0][pos], dsp.ring[1][pos]);
                    dsp.ring[0][pos] = in_l[i];
                    dsp.ring[1][pos] = in_r[i];
                    dsp.pos = (pos + 1) % ECHO;
                    out
                }
                1 | 2 => {
                    let v = if dsp.clock.is_multiple_of(CLICK_PERIOD) {
                        0.5
                    } else {
                        0.0
                    };
                    (v, v)
                }
                _ => (0.5, 0.5),
            };
            dsp.clock += 1;
            silent &= l == 0.0 && r == 0.0;
            out_l[i] = l;
            out_r[i] = r;
        }
        let flags = match kind {
            0 if silent => 0b11,
            3 if in_flags & 0b11 == 0b11 => 0b11,
            _ => 0,
        };
        if data.numOutputs > 0 && !data.outputs.is_null() {
            (*data.outputs).silenceFlags = flags;
        }
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> u32 {
        match self.kind.load(Ordering::Relaxed) {
            0 => ECHO as u32,
            1 => kInfiniteTail,
            _ => kNoTail,
        }
    }
}

impl IEditControllerTrait for TailFx {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> i32 {
        4
    }
    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        let info = &mut *info;
        let hidden = (kIsHidden | kIsReadOnly) as u32;
        match index {
            0 => param_info(info, KIND, "Kind", 3, 0.0, kCanAutomate as u32),
            1 => param_info(info, CALLS, "Calls", 0, 0.0, hidden),
            2 => param_info(info, IN_FLAGS, "InFlags", 0, 0.0, hidden),
            3 => param_info(info, COST, "Cost", 0, 0.0, kCanAutomate as u32),
            _ => return kInvalidArgument,
        }
        kResultOk
    }
    unsafe fn getParamStringByValue(
        &self,
        _id: ParamID,
        value: ParamValue,
        string: *mut String128,
    ) -> tresult {
        copy_wstring(&format!("{value:.2}"), &mut *string);
        kResultOk
    }
    unsafe fn getParamValueByString(
        &self,
        _id: ParamID,
        _string: *mut TChar,
        _value: *mut ParamValue,
    ) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value
    }
    unsafe fn plainParamToNormalized(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value
    }
    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        match id {
            KIND => f64::from(self.kind.load(Ordering::Relaxed)) / 3.0,
            CALLS => self.calls.load(Ordering::Relaxed) as f64 / 1e6,
            IN_FLAGS => self.in_flags.load(Ordering::Relaxed) as f64 / 3.0,
            COST => load_f64(&self.cost),
            _ => 0.0,
        }
    }
    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        // Single component: the controller value is the processor value (the read-only
        // reports ignore what the host echoes back).
        if self.set(id, value) || matches!(id, CALLS | IN_FLAGS) {
            kResultOk
        } else {
            kInvalidArgument
        }
    }
    unsafe fn setComponentHandler(&self, _handler: *mut IComponentHandler) -> tresult {
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        std::ptr::null_mut()
    }
}

// ---------------------------------------------------------------------------------------
// factory + entry points

struct Factory {
    _live: Live,
}

impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}

const CLASSES: [(TUID, &str, &str, &str); 4] = [
    (
        EFFECT_CID,
        "Audio Module Class",
        "Ether VST3 Gain",
        "Fx|Dynamics",
    ),
    (
        INSTRUMENT_CID,
        "Audio Module Class",
        "Ether VST3 Synth",
        "Instrument|Synth",
    ),
    (
        TAIL_CID,
        "Audio Module Class",
        "Ether VST3 Tail",
        "Fx|Delay",
    ),
    (
        EFFECT_CONTROLLER_CID,
        "Component Controller Class",
        "Ether VST3 Gain",
        "",
    ),
];

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        let info = &mut *info;
        copy_cstring("Ethereal", &mut info.vendor);
        copy_cstring("https://github.com/diegoimbert/ethereal", &mut info.url);
        copy_cstring("", &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as i32;
        kResultOk
    }
    unsafe fn countClasses(&self) -> i32 {
        CLASSES.len() as i32
    }
    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        let Some((cid, category, name, _)) = CLASSES.get(index as usize) else {
            return kInvalidArgument;
        };
        let info = &mut *info;
        info.cid = *cid;
        info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
        copy_cstring(category, &mut info.category);
        copy_cstring(name, &mut info.name);
        kResultOk
    }
    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        let instance = match *(cid as *const TUID) {
            EFFECT_CID => ComWrapper::new(EffectProcessor::new()).to_com_ptr::<FUnknown>(),
            INSTRUMENT_CID => ComWrapper::new(Instrument::new()).to_com_ptr::<FUnknown>(),
            TAIL_CID => ComWrapper::new(TailFx::new()).to_com_ptr::<FUnknown>(),
            EFFECT_CONTROLLER_CID => {
                ComWrapper::new(EffectController::new()).to_com_ptr::<FUnknown>()
            }
            _ => None,
        };
        match instance {
            Some(i) => {
                let ptr = i.as_ptr();
                ((*(*ptr).vtbl).queryInterface)(ptr, iid as *const TUID, obj)
            }
            None => kInvalidArgument,
        }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        let Some((cid, category, name, sub)) = CLASSES.get(index as usize) else {
            return kInvalidArgument;
        };
        let info = &mut *info;
        info.cid = *cid;
        info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as i32;
        copy_cstring(category, &mut info.category);
        copy_cstring(name, &mut info.name);
        info.classFlags = 0;
        copy_cstring(sub, &mut info.subCategories);
        copy_cstring("Ethereal", &mut info.vendor);
        copy_cstring("1.2.3", &mut info.version);
        copy_cstring("VST 3.7", &mut info.sdkVersion);
        kResultOk
    }
}

/// Crash or hang on demand, based on this module's own path (see the module docs).
fn misbehave() {
    let act = |path: &str| {
        if path.contains("ether-crash") {
            std::process::abort();
        }
        if path.contains("ether-hang") {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    };
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void)
            -> i32;
            fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
        }
        // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT
        const FLAGS: u32 = 0x4 | 0x2;
        let mut module = std::ptr::null_mut();
        // SAFETY: an address inside this module; the handle is not reference-counted.
        if unsafe { GetModuleHandleExW(FLAGS, GetPluginFactory as *const u16, &mut module) } == 0 {
            return;
        }
        let mut name = [0u16; 2048];
        // SAFETY: `name` holds `name.len()` UTF-16 units.
        let n =
            unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
        act(&String::from_utf16_lossy(&name[..n.min(name.len())]));
    }
    #[cfg(unix)]
    {
        #[repr(C)]
        struct DlInfo {
            fname: *const c_char,
            fbase: *mut c_void,
            sname: *const c_char,
            saddr: *mut c_void,
        }
        unsafe extern "C" {
            fn dladdr(addr: *const c_void, info: *mut DlInfo) -> i32;
        }
        let mut info: DlInfo = unsafe { std::mem::zeroed() };
        let found = unsafe { dladdr(GetPluginFactory as *const c_void, &mut info) } != 0;
        if !found || info.fname.is_null() {
            return;
        }
        act(&unsafe { CStr::from_ptr(info.fname) }.to_string_lossy());
    }
}

#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
extern "system" fn InitDll() -> bool {
    module_entry()
}

#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
extern "system" fn ExitDll() -> bool {
    module_exit()
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
extern "system" fn bundleEntry(_bundle: *mut c_void) -> bool {
    module_entry()
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
extern "system" fn bundleExit() -> bool {
    module_exit()
}

#[cfg(all(unix, not(target_os = "macos")))]
#[unsafe(no_mangle)]
extern "system" fn ModuleEntry(_handle: *mut c_void) -> bool {
    module_entry()
}

#[cfg(all(unix, not(target_os = "macos")))]
#[unsafe(no_mangle)]
extern "system" fn ModuleExit() -> bool {
    module_exit()
}

#[unsafe(no_mangle)]
extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    misbehave();
    ComWrapper::new(Factory { _live: Live::new() })
        .to_com_ptr::<IPluginFactory>()
        .map_or(std::ptr::null_mut(), |p| p.into_raw())
}
