//! The host side of the VST2 ABI: `audioMasterCallback` and the per-instance state it talks
//! to.
//!
//! # Finding the instance
//! - While the host is inside a plugin call on the **audio thread** (`processReplacing`,
//!   `effProcessEvents`, `setParameter` from the node), the node publishes an [`AudioScope`]
//!   in a thread-local ([`AudioScope::enter`]). Audio-thread callbacks (`GetTime`,
//!   `ProcessEvents` for MIDI out, `Automate` echoes) are answered from it without locks or
//!   allocation.
//! - Elsewhere the instance is found through `AEffect::resvd2` (reserved for the host by the
//!   ABI; JUCE uses it the same way), which points at the instance's [`HostShared`].
//! - Before the entry point returned (`VSTPluginMain`), there is no instance yet: only
//!   stateless queries are answered, plus `audioMasterCurrentId` (the shell sub-plugin being
//!   created, from [`with_current_id`]).
//!
//! # Threads
//! `HostShared` is touched from the plugin main thread (the controller), the plugin's own GUI
//! threads, and (atomics only) the audio thread. GUI edits (`audioMasterAutomate`, `Begin/
//! EndEdit` outside the audio thread) queue under a mutex that the audio thread never takes.

use std::cell::{Cell, UnsafeCell};
use std::ffi::{CStr, CString, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::abi::*;

/// A GUI edit reported by the plugin (main/GUI thread), drained by `poll`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Edit {
    Begin(u32),
    End(u32),
    Perform(u32, f32),
}

/// Host state of one plugin instance (see the module docs).
pub(crate) struct HostShared {
    edits: Mutex<Vec<Edit>>,
    /// > 0 while the host itself changes params on the main thread (`set_param_value`,
    /// `load_state`): `audioMasterAutomate` echoes are not user edits then.
    suppress: AtomicU32,
    io_changed: AtomicBool,
    update_display: AtomicBool,
    /// Editor resize request `(width << 32) | height`, `u64::MAX` = none.
    resize: AtomicU64,
    sample_rate: AtomicU32,
    block_size: AtomicU32,
    /// Transport answered to `audioMasterGetTime` outside the audio thread (stopped, at the
    /// current sample rate). Written on the main thread at activation only.
    idle_time: UnsafeCell<VstTimeInfo>,
    /// `audioMasterGetDirectory`: the plugin's folder.
    directory: CString,
}

// SAFETY: every field is atomic or a mutex, except `idle_time` (written only by the main
// thread while no plugin call is running; plugins only read it) and `directory` (immutable).
unsafe impl Send for HostShared {}
unsafe impl Sync for HostShared {}

impl std::fmt::Debug for HostShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostShared")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

const NO_RESIZE: u64 = u64::MAX;

impl HostShared {
    pub fn new(directory: &std::path::Path) -> Self {
        let directory = CString::new(directory.to_string_lossy().as_bytes()).unwrap_or_default();
        Self {
            edits: Mutex::new(Vec::new()),
            suppress: AtomicU32::new(0),
            io_changed: AtomicBool::new(false),
            update_display: AtomicBool::new(false),
            resize: AtomicU64::new(NO_RESIZE),
            sample_rate: AtomicU32::new(48_000f32.to_bits()),
            block_size: AtomicU32::new(512),
            idle_time: UnsafeCell::new(idle_time(48_000.0)),
            directory,
        }
    }

    /// Main thread, no plugin call running: the processing setup the plugin may query.
    pub fn set_setup(&self, sample_rate: f32, block_size: u32) {
        self.sample_rate
            .store(sample_rate.to_bits(), Ordering::Relaxed);
        self.block_size.store(block_size, Ordering::Relaxed);
        // SAFETY: main thread, outside any plugin call (see the field docs).
        unsafe { *self.idle_time.get() = idle_time(f64::from(sample_rate)) };
    }

    pub fn sample_rate(&self) -> f32 {
        f32::from_bits(self.sample_rate.load(Ordering::Relaxed))
    }

    /// Run `f` (a host-initiated param/state change) with automation echoes ignored.
    pub fn suppressed<R>(&self, f: impl FnOnce() -> R) -> R {
        self.suppress.fetch_add(1, Ordering::AcqRel);
        let r = f();
        self.suppress.fetch_sub(1, Ordering::AcqRel);
        r
    }

    pub fn take_edits(&self) -> Vec<Edit> {
        std::mem::take(&mut *self.edits.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn take_io_changed(&self) -> bool {
        self.io_changed.swap(false, Ordering::AcqRel)
    }

    pub fn take_update_display(&self) -> bool {
        self.update_display.swap(false, Ordering::AcqRel)
    }

    /// The last `audioMasterSizeWindow` request, `(width, height)`.
    pub fn take_resize(&self) -> Option<(u32, u32)> {
        let v = self.resize.swap(NO_RESIZE, Ordering::AcqRel);
        (v != NO_RESIZE).then_some(((v >> 32) as u32, v as u32))
    }

    fn edit(&self, edit: Edit) {
        if self.suppress.load(Ordering::Acquire) > 0 {
            return;
        }
        self.edits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(edit);
    }
}

fn idle_time(sample_rate: f64) -> VstTimeInfo {
    VstTimeInfo {
        sample_rate,
        tempo: 120.0,
        time_sig_numerator: 4,
        time_sig_denominator: 4,
        flags: kVstTempoValid | kVstTimeSigValid | kVstPpqPosValid,
        ..VstTimeInfo::default()
    }
}

/// MIDI the plugin sends during processing (`audioMasterProcessEvents`), pre-allocated by the
/// node: `(sample offset in the block, bytes)`.
pub(crate) struct MidiOut {
    pub events: Box<[(u32, [u8; 3])]>,
    pub len: usize,
    /// Block offset of the sub-block being processed (plugin deltas are relative to it).
    pub base: u32,
}

impl MidiOut {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            events: vec![(0, [0; 3]); n.max(1)].into_boxed_slice(),
            len: 0,
            base: 0,
        }
    }

    fn push(&mut self, offset: u32, data: [u8; 3]) {
        if let Some(slot) = self.events.get_mut(self.len) {
            *slot = (offset, data);
            self.len += 1;
        }
    }
}

/// What audio-thread callbacks see while the node is inside a plugin call.
pub(crate) struct AudioScope {
    pub time: *mut VstTimeInfo,
    pub midi_out: *mut MidiOut,
}

thread_local! {
    static AUDIO: Cell<*const AudioScope> = const { Cell::new(std::ptr::null()) };
    static CURRENT_ID: Cell<i32> = const { Cell::new(0) };
}

/// Restores the previous audio scope on drop.
pub(crate) struct ScopeGuard(*const AudioScope);

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        AUDIO.with(|a| a.set(self.0));
    }
}

impl AudioScope {
    /// Publish `self` to this thread's callbacks until the guard drops. RT-safe.
    pub fn enter(&self) -> ScopeGuard {
        ScopeGuard(AUDIO.with(|a| a.replace(self)))
    }
}

/// Run `f` with `audioMasterCurrentId` answering `id` (shell sub-plugin creation; 0 = the
/// default plugin of the library).
pub(crate) fn with_current_id<R>(id: i32, f: impl FnOnce() -> R) -> R {
    let prev = CURRENT_ID.with(|c| c.replace(id));
    let r = f();
    CURRENT_ID.with(|c| c.set(prev));
    r
}

/// Host capabilities answered "yes" to `audioMasterCanDo`.
const CAN_DO: &[&[u8]] = &[
    b"sendVstEvents",
    b"sendVstMidiEvent",
    b"sendVstTimeInfo",
    b"receiveVstEvents",
    b"receiveVstMidiEvent",
    b"sizeWindow",
    b"supplyIdle",
    b"shellCategory",
    b"shellCategorycurID",
    b"startStopProcess",
];

const HOST_NAME: &[u8] = b"Ethereal\0";

/// Write `s` (NUL-terminated) into a plugin-provided buffer of at least 64 bytes.
unsafe fn write_cstr(ptr: *mut c_void, s: &[u8]) {
    let n = s.len().min(63);
    // SAFETY: the ABI guarantees the vendor/product buffers hold 64 bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), ptr.cast::<u8>(), n);
        *ptr.cast::<u8>().add(n) = 0;
    }
}

/// `audioMasterCallback` for every instance. Never panics (it is called from C).
pub(crate) unsafe extern "C" fn host_callback(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: VstIntPtr,
    ptr: *mut c_void,
    opt: f32,
) -> VstIntPtr {
    let audio = AUDIO.with(Cell::get);
    // SAFETY: a non-null scope is alive for the duration of the plugin call that brought us
    // here (see `AudioScope::enter`).
    let audio = unsafe { audio.as_ref() };
    // SAFETY: `resvd2` is ours: 0 or a `HostShared` kept alive by the `Effect` that set it.
    let host = unsafe {
        effect
            .as_ref()
            .and_then(|e| (e.resvd2 as *const HostShared).as_ref())
    };
    match opcode {
        audioMasterVersion => VST_VERSION as VstIntPtr,
        audioMasterCurrentId => CURRENT_ID.with(Cell::get) as VstIntPtr,
        audioMasterIdle | audioMasterNeedIdle | audioMasterWantMidi => 1,
        audioMasterAutomate => {
            // Values the plugin changes while processing are its own business (VST2 has one
            // object: its GUI already follows). Elsewhere it is a GUI edit.
            if audio.is_none()
                && let Some(h) = host
                && index >= 0
            {
                h.edit(Edit::Perform(index as u32, opt));
            }
            0
        }
        audioMasterBeginEdit | audioMasterEndEdit => {
            if audio.is_none()
                && let Some(h) = host
                && index >= 0
            {
                let i = index as u32;
                h.edit(if opcode == audioMasterBeginEdit {
                    Edit::Begin(i)
                } else {
                    Edit::End(i)
                });
            }
            1
        }
        audioMasterGetTime => match (audio, host) {
            (Some(a), _) => a.time as VstIntPtr,
            (None, Some(h)) => h.idle_time.get() as VstIntPtr,
            _ => 0,
        },
        audioMasterTempoAt => {
            // SAFETY: the scope's time info is alive during the call.
            let bpm = audio.map_or(120.0, |a| unsafe { (*a.time).tempo });
            (bpm * 10_000.0) as VstIntPtr
        }
        audioMasterProcessEvents => {
            if let (Some(a), false) = (audio, ptr.is_null()) {
                // SAFETY: the plugin passes a valid `VstEvents` with `num_events` pointers;
                // the scope's MIDI buffer is the node's and only used on this thread.
                unsafe { collect_midi(ptr.cast::<VstEvents>(), &mut *a.midi_out) };
            }
            1
        }
        audioMasterIOChanged => {
            if let Some(h) = host {
                h.io_changed.store(true, Ordering::Release);
            }
            1
        }
        audioMasterUpdateDisplay => {
            if let Some(h) = host {
                h.update_display.store(true, Ordering::Release);
            }
            1
        }
        audioMasterSizeWindow => match host {
            Some(h) if index > 0 && value > 0 => {
                let packed = (u64::from(index as u32) << 32) | u64::from(value as u32);
                h.resize.store(packed, Ordering::Release);
                1
            }
            _ => 0,
        },
        audioMasterGetSampleRate => host.map_or(48_000.0, |h| h.sample_rate()) as VstIntPtr,
        audioMasterGetBlockSize => {
            host.map_or(512, |h| h.block_size.load(Ordering::Relaxed)) as VstIntPtr
        }
        audioMasterGetInputLatency | audioMasterGetOutputLatency => 0,
        audioMasterGetCurrentProcessLevel => {
            if audio.is_some() {
                kVstProcessLevelRealtime
            } else {
                kVstProcessLevelUser
            }
        }
        audioMasterGetAutomationState => 1, // "off": no automation recording state to report
        audioMasterGetVendorString | audioMasterGetProductString => {
            if ptr.is_null() {
                return 0;
            }
            // SAFETY: see `write_cstr`.
            unsafe { write_cstr(ptr, &HOST_NAME[..HOST_NAME.len() - 1]) };
            1
        }
        audioMasterGetVendorVersion => 1,
        audioMasterGetLanguage => kVstLangEnglish,
        audioMasterGetDirectory => host.map_or(0, |h| h.directory.as_ptr() as VstIntPtr),
        audioMasterCanDo => {
            if ptr.is_null() {
                return 0;
            }
            // SAFETY: the plugin passes a NUL-terminated string.
            let what = unsafe { CStr::from_ptr(ptr.cast()) }.to_bytes();
            VstIntPtr::from(CAN_DO.contains(&what))
        }
        _ => {
            let _ = value;
            0
        }
    }
}

/// Copy the short MIDI events of `events` into `out` (sysex is dropped).
unsafe fn collect_midi(events: *const VstEvents, out: &mut MidiOut) {
    // SAFETY: caller guarantees a valid `VstEvents` header.
    let n = unsafe { (*events).num_events }.max(0) as usize;
    // SAFETY: the pointer array follows the header (`events[num_events]`).
    let list = unsafe { std::ptr::addr_of!((*events).events).cast::<*const VstEvent>() };
    for i in 0..n {
        // SAFETY: `i < num_events`; each pointer is a valid event per the ABI.
        let e = unsafe { *list.add(i) };
        let Some(header) = (unsafe { e.as_ref() }) else {
            continue;
        };
        if header.event_type != kVstMidiType {
            continue;
        }
        // SAFETY: a `kVstMidiType` event is a `VstMidiEvent`.
        let m = unsafe { &*e.cast::<VstMidiEvent>() };
        let offset = out.base + m.delta_frames.max(0) as u32;
        out.push(offset, [m.midi_data[0], m.midi_data[1], m.midi_data[2]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(effect: *mut AEffect, op: i32, index: i32, value: isize, ptr: *mut c_void) -> isize {
        // SAFETY: test calls with valid or null pointers.
        unsafe { host_callback(effect, op, index, value, ptr, 0.0) }
    }

    #[test]
    fn stateless_queries() {
        let null = std::ptr::null_mut::<c_void>();
        let no_fx = std::ptr::null_mut::<AEffect>();
        assert_eq!(call(no_fx, audioMasterVersion, 0, 0, null), 2400);
        assert_eq!(call(no_fx, audioMasterCurrentId, 0, 0, null), 0);
        with_current_id(42, || {
            assert_eq!(call(no_fx, audioMasterCurrentId, 0, 0, null), 42);
        });
        assert_eq!(call(no_fx, audioMasterCurrentId, 0, 0, null), 0);
        let can = |s: &CStr| call(no_fx, audioMasterCanDo, 0, 0, s.as_ptr() as *mut c_void);
        assert_eq!(can(c"sendVstTimeInfo"), 1);
        assert_eq!(can(c"shellCategory"), 1);
        assert_eq!(can(c"offline"), 0);
        let mut buf = [0xFFu8; 64];
        assert_eq!(
            call(no_fx, audioMasterGetVendorString, 0, 0, buf.as_mut_ptr().cast()),
            1
        );
        assert_eq!(read_cstr(&buf), "Ethereal");
        assert_eq!(call(no_fx, audioMasterGetTime, 0, 0, null), 0);
        assert_eq!(call(no_fx, 9999, 0, 0, null), 0);
    }

    #[test]
    fn instance_queries_and_edits() {
        let shared = HostShared::new(std::path::Path::new("/plugins"));
        shared.set_setup(44_100.0, 256);
        // SAFETY: all-zero is a valid AEffect for the host side (no function is called).
        let mut effect: AEffect = unsafe { std::mem::zeroed() };
        effect.resvd2 = &shared as *const HostShared as isize;
        let e = &mut effect as *mut AEffect;
        let null = std::ptr::null_mut();
        assert_eq!(call(e, audioMasterGetSampleRate, 0, 0, null), 44_100);
        assert_eq!(call(e, audioMasterGetBlockSize, 0, 0, null), 256);
        assert_eq!(call(e, audioMasterGetCurrentProcessLevel, 0, 0, null), 1);
        let dir = call(e, audioMasterGetDirectory, 0, 0, null) as *const std::ffi::c_char;
        // SAFETY: points at `shared.directory`.
        assert_eq!(unsafe { CStr::from_ptr(dir) }, c"/plugins");
        let t = call(e, audioMasterGetTime, 0, 0, null) as *const VstTimeInfo;
        // SAFETY: points at `shared.idle_time`.
        assert_eq!(unsafe { (*t).sample_rate }, 44_100.0);

        call(e, audioMasterBeginEdit, 3, 0, null);
        // SAFETY: as `call`.
        unsafe { host_callback(e, audioMasterAutomate, 3, 0, null, 0.25) };
        call(e, audioMasterEndEdit, 3, 0, null);
        shared.suppressed(|| unsafe { host_callback(e, audioMasterAutomate, 1, 0, null, 0.5) });
        assert_eq!(
            shared.take_edits(),
            [Edit::Begin(3), Edit::Perform(3, 0.25), Edit::End(3)]
        );
        assert!(shared.take_edits().is_empty());

        assert_eq!(call(e, audioMasterSizeWindow, 300, 200, null), 1);
        assert_eq!(shared.take_resize(), Some((300, 200)));
        assert_eq!(shared.take_resize(), None);
        call(e, audioMasterIOChanged, 0, 0, null);
        assert!(shared.take_io_changed());
        assert!(!shared.take_io_changed());
        call(e, audioMasterUpdateDisplay, 0, 0, null);
        assert!(shared.take_update_display());
    }

    #[test]
    fn audio_scope_answers_time_and_collects_midi() {
        let shared = HostShared::new(std::path::Path::new("/"));
        // SAFETY: as above.
        let mut effect: AEffect = unsafe { std::mem::zeroed() };
        effect.resvd2 = &shared as *const HostShared as isize;
        let e = &mut effect as *mut AEffect;
        let mut time = VstTimeInfo {
            tempo: 140.0,
            ..VstTimeInfo::default()
        };
        let mut midi = MidiOut::with_capacity(4);
        midi.base = 10;
        let scope = AudioScope {
            time: &mut time,
            midi_out: &mut midi,
        };
        let mut ev = VstMidiEvent {
            event_type: kVstMidiType,
            byte_size: 32,
            delta_frames: 5,
            midi_data: [0x90, 60, 100, 0],
            ..VstMidiEvent::default()
        };
        let mut events = VstEventsBuf::<1> {
            num_events: 1,
            reserved: 0,
            events: [(&mut ev as *mut VstMidiEvent).cast()],
        };
        let null = std::ptr::null_mut();
        {
            let _g = scope.enter();
            let t = call(e, audioMasterGetTime, 0, 0, null) as *const VstTimeInfo;
            // SAFETY: points at `time`.
            assert_eq!(unsafe { (*t).tempo }, 140.0);
            assert_eq!(call(e, audioMasterTempoAt, 0, 0, null), 1_400_000);
            assert_eq!(call(e, audioMasterGetCurrentProcessLevel, 0, 0, null), 2);
            call(e, audioMasterProcessEvents, 0, 0, (&mut events as *mut VstEventsBuf<1>).cast());
            // Automation while processing is not a GUI edit.
            unsafe { host_callback(e, audioMasterAutomate, 0, 0, null, 1.0) };
        }
        assert_eq!(midi.len, 1);
        assert_eq!(midi.events[0], (15, [0x90, 60, 100]));
        assert!(shared.take_edits().is_empty());
        // Scope left: back to the idle transport.
        let t = call(e, audioMasterGetTime, 0, 0, null) as *const VstTimeInfo;
        assert_eq!(unsafe { (*t).tempo }, 120.0);
    }
}
