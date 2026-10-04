//! Minimal VST2 test plugins for Ethereal's VST2 host tests, built from `ether_vst2::abi`
//! (no Steinberg SDK). The same source builds two libraries (two `[[example]]`s, told apart
//! by `CARGO_CRATE_NAME`):
//!
//! - `ether_vst2_test_plugin`: **Ether VST2 Gain** (`'EtG2'`): stereo gain effect.
//!   - params: 0 `Gain` (out = in · 2 · gain; default 0.5 = unity, display in dB),
//!     1 `Mode` (stored only), 2 `Tempo` (the plugin writes `tempo / 1000` from
//!     `audioMasterGetTime` every block);
//!   - `processReplacing` + `processDoubleReplacing`, program chunks (`effGetChunk` /
//!     `effSetChunk`, magic `EG2C`), 2 programs, latency (`initialDelay`) 32;
//!   - `effVendorSpecific` triggers: index 1 = a GUI-style edit of `Gain` to 0.75
//!     (`BeginEdit`, `Automate`, `EndEdit`); 2 = latency becomes 128 + `audioMasterIOChanged`;
//!     3 = `audioMasterUpdateDisplay`; 4 = `audioMasterSizeWindow(300, 200)`.
//! - `ether_vst2_test_shell`: a **shell** (`kPlugCategShell`) with two sub-plugins chosen by
//!   `audioMasterCurrentId`:
//!   - **Ether Shell Gain** (`'EtSg'`): the gain effect with `processDoubleReplacing` only,
//!     no chunks (state through param values), no latency;
//!   - **Ether Shell Synth** (`'EtSy'`): an instrument (`effFlagsIsSynth`, no inputs) whose
//!     output is a DC level `2 · Volume · velocity` while a note is held (sample-accurate),
//!     echoing note-ons back to the host (`audioMasterProcessEvents`); claims an editor
//!     (`effEditGetRect` 200x100, no real view).
//!
//! Loading aborts the process if the library path contains `ether-crash`, and hangs forever
//! if it contains `ether-hang` (scanner crash/timeout tests; Unix only).
//!
//! Nothing here allocates on the audio thread.

#![allow(clippy::missing_safety_doc, non_upper_case_globals)]

use std::ffi::c_void;

use ether_vst2::abi::*;

const GAIN_UID: i32 = fourcc(*b"EtG2");
const SHELL_GAIN_UID: i32 = fourcc(*b"EtSg");
const SYNTH_UID: i32 = fourcc(*b"EtSy");
const SHELL_UID: i32 = fourcc(*b"EtSh");
const CHUNK_MAGIC: &[u8; 4] = b"EG2C";
const MAX_EVENTS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gain,
    ShellGain,
    Synth,
    Shell,
}

/// The instance. `effect` is first, so an `*mut AEffect` is an `*mut Plugin`.
#[repr(C)]
struct Plugin {
    effect: AEffect,
    host: HostCallback,
    kind: Kind,
    params: [f32; 3],
    program: i32,
    chunk: Vec<u8>,
    shell_index: usize,
    rect: ERect,
    /// Synth: MIDI received for the next block `(delta, bytes)`.
    events: [(i32, [u8; 3]); MAX_EVENTS],
    num_events: usize,
    level: f32,
}

fn is_shell_build() -> bool {
    env!("CARGO_CRATE_NAME").ends_with("shell")
}

const SHELL_PLUGINS: [(i32, &str); 2] = [
    (SHELL_GAIN_UID, "Ether Shell Gain"),
    (SYNTH_UID, "Ether Shell Synth"),
];

unsafe fn write_str(ptr: *mut c_void, s: &str) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: hosts pass buffers of at least 64 bytes for these strings (Ethereal: 256).
    unsafe {
        let n = s.len().min(63);
        std::ptr::copy_nonoverlapping(s.as_ptr(), ptr.cast::<u8>(), n);
        *ptr.cast::<u8>().add(n) = 0;
    }
}

impl Plugin {
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Gain => "Ether VST2 Gain",
            Kind::ShellGain => "Ether Shell Gain",
            Kind::Synth => "Ether Shell Synth",
            Kind::Shell => "Ether Shell",
        }
    }

    fn param_name(&self, i: i32) -> &'static str {
        match (self.kind, i) {
            (Kind::Synth, 0) => "Volume",
            (_, 0) => "Gain",
            (_, 1) => "Mode",
            (_, 2) => "Tempo",
            _ => "",
        }
    }

    fn call_host(&mut self, op: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32) -> isize {
        // SAFETY: the host's callback with our own effect.
        unsafe { (self.host)(&mut self.effect, op, index, value, ptr, opt) }
    }

    /// Read the tempo from the host transport into the `Tempo` param.
    fn read_time(&mut self) {
        let t = self.call_host(audioMasterGetTime, 0, kVstTempoValid as isize, std::ptr::null_mut(), 0.0)
            as *const VstTimeInfo;
        // SAFETY: the host returns null or a valid time info.
        if let Some(t) = unsafe { t.as_ref() }
            && t.flags & kVstTempoValid != 0
        {
            self.params[2] = (t.tempo / 1000.0) as f32;
        }
    }

    fn gain(&self) -> f32 {
        self.params[0] * 2.0
    }

    fn encode_chunk(&mut self) {
        self.chunk.clear();
        self.chunk.extend_from_slice(CHUNK_MAGIC);
        self.chunk.extend_from_slice(&self.program.to_le_bytes());
        for p in self.params {
            self.chunk.extend_from_slice(&p.to_le_bytes());
        }
    }

    fn decode_chunk(&mut self, data: &[u8]) -> bool {
        let Some(rest) = data.strip_prefix(CHUNK_MAGIC) else {
            return false;
        };
        if rest.len() != 4 + 4 * self.params.len() {
            return false;
        }
        self.program = i32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        for (i, b) in rest[4..].chunks_exact(4).enumerate() {
            self.params[i] = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        true
    }

    fn render_synth(&mut self, outputs: *mut *mut f32, frames: usize) {
        let mut next = 0;
        // SAFETY: two output channels of `frames` samples.
        let (l, r) = unsafe {
            (
                std::slice::from_raw_parts_mut(*outputs, frames),
                std::slice::from_raw_parts_mut(*outputs.add(1), frames),
            )
        };
        for i in 0..frames {
            while next < self.num_events && self.events[next].0 as usize <= i {
                let [status, _key, vel] = self.events[next].1;
                match status & 0xF0 {
                    0x90 if vel > 0 => self.level = 2.0 * self.params[0] * f32::from(vel) / 127.0,
                    0x80 | 0x90 => self.level = 0.0,
                    0xB0 => self.level = 0.0, // all notes off & co.
                    _ => {}
                }
                next += 1;
            }
            l[i] = self.level;
            r[i] = self.level;
        }
        self.num_events = 0;
    }
}

fn plugin<'a>(e: *mut AEffect) -> &'a mut Plugin {
    // SAFETY: every AEffect this library hands out is the first field of a `Plugin`.
    unsafe { &mut *e.cast::<Plugin>() }
}

unsafe extern "C" fn dispatcher(
    e: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize {
    let p = plugin(e);
    match opcode {
        effClose => {
            // SAFETY: allocated by `VSTPluginMain` with `Box::into_raw`.
            drop(unsafe { Box::from_raw(e.cast::<Plugin>()) });
            0
        }
        effSetProgram => {
            p.program = value as i32;
            0
        }
        effGetProgram => p.program as isize,
        effGetParamName => {
            unsafe { write_str(ptr, p.param_name(index)) };
            0
        }
        effGetParamLabel => {
            let label = if p.kind != Kind::Synth && index == 0 { "dB" } else { "" };
            unsafe { write_str(ptr, label) };
            0
        }
        effGetParamDisplay => {
            let v = p.params.get(index as usize).copied().unwrap_or(0.0);
            let text = if p.kind != Kind::Synth && index == 0 {
                let g = v * 2.0;
                if g <= 0.0 {
                    "-inf".to_owned()
                } else {
                    format!("{:.1}", 20.0 * g.log10())
                }
            } else {
                format!("{v:.2}")
            };
            unsafe { write_str(ptr, &text) };
            0
        }
        effGetEffectName | effGetProductString => {
            unsafe { write_str(ptr, p.name()) };
            1
        }
        effGetVendorString => {
            unsafe { write_str(ptr, "Ethereal") };
            1
        }
        effGetVendorVersion => 1203,
        effGetVstVersion => VST_VERSION as isize,
        effGetPlugCategory => match p.kind {
            Kind::Gain | Kind::ShellGain => kPlugCategEffect,
            Kind::Synth => kPlugCategSynth,
            Kind::Shell => kPlugCategShell,
        },
        effShellGetNextPlugin => match SHELL_PLUGINS.get(p.shell_index) {
            Some((uid, name)) if p.kind == Kind::Shell => {
                p.shell_index += 1;
                unsafe { write_str(ptr, name) };
                *uid as isize
            }
            _ => 0,
        },
        effCanDo => {
            if ptr.is_null() {
                return 0;
            }
            // SAFETY: the host passes a C string.
            let what = unsafe { std::ffi::CStr::from_ptr(ptr.cast()) }.to_bytes();
            match (p.kind, what) {
                (Kind::Synth, b"receiveVstMidiEvent" | b"receiveVstEvents") => 1,
                (Kind::Synth, b"sendVstMidiEvent" | b"sendVstEvents") => 1,
                _ => -1,
            }
        }
        effGetTailSize => {
            if p.kind == Kind::Synth {
                1 // no tail
            } else {
                4800
            }
        }
        effGetChunk => {
            if ptr.is_null() || p.effect.flags & effFlagsProgramChunks == 0 {
                return 0;
            }
            p.encode_chunk();
            // SAFETY: the host passes a `void**`.
            unsafe { *ptr.cast::<*mut c_void>() = p.chunk.as_mut_ptr().cast() };
            p.chunk.len() as isize
        }
        effSetChunk => {
            if ptr.is_null() || value <= 0 {
                return 0;
            }
            // SAFETY: `value` bytes at `ptr`, per the ABI.
            let data = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), value as usize) };
            isize::from(p.decode_chunk(data))
        }
        effProcessEvents => {
            if ptr.is_null() {
                return 0;
            }
            let events = ptr.cast::<VstEvents>();
            // SAFETY: a valid VstEvents with `num_events` pointers.
            let n = unsafe { (*events).num_events }.max(0) as usize;
            let list = unsafe { std::ptr::addr_of!((*events).events).cast::<*const VstEvent>() };
            for i in 0..n {
                let ev = unsafe { *list.add(i) };
                // SAFETY: valid event pointers.
                if ev.is_null() || unsafe { (*ev).event_type } != kVstMidiType {
                    continue;
                }
                let m = unsafe { &*ev.cast::<VstMidiEvent>() };
                if p.num_events < MAX_EVENTS {
                    p.events[p.num_events] =
                        (m.delta_frames, [m.midi_data[0], m.midi_data[1], m.midi_data[2]]);
                    p.num_events += 1;
                }
            }
            1
        }
        effEditGetRect => {
            if ptr.is_null() || p.effect.flags & effFlagsHasEditor == 0 {
                return 0;
            }
            // SAFETY: the host passes an `ERect**`.
            unsafe { *ptr.cast::<*mut ERect>() = &mut p.rect };
            1
        }
        effEditOpen | effEditClose | effEditIdle => 1,
        effVendorSpecific => {
            let null = std::ptr::null_mut();
            match index {
                1 => {
                    p.call_host(audioMasterBeginEdit, 0, 0, null, 0.0);
                    p.params[0] = 0.75;
                    p.call_host(audioMasterAutomate, 0, 0, null, 0.75);
                    p.call_host(audioMasterEndEdit, 0, 0, null, 0.0);
                    1
                }
                2 => {
                    p.effect.initial_delay = 128;
                    p.call_host(audioMasterIOChanged, 0, 0, null, 0.0);
                    1
                }
                3 => p.call_host(audioMasterUpdateDisplay, 0, 0, null, 0.0),
                4 => p.call_host(audioMasterSizeWindow, 300, 200, null, 0.0),
                _ => 0,
            }
        }
        _ => {
            let _ = opt;
            0
        }
    }
}

unsafe extern "C" fn set_parameter(e: *mut AEffect, index: i32, value: f32) {
    let p = plugin(e);
    if let Some(v) = p.params.get_mut(index as usize) {
        *v = value;
    }
}

unsafe extern "C" fn get_parameter(e: *mut AEffect, index: i32) -> f32 {
    plugin(e).params.get(index as usize).copied().unwrap_or(0.0)
}

unsafe extern "C" fn process_replacing(
    e: *mut AEffect,
    inputs: *mut *mut f32,
    outputs: *mut *mut f32,
    frames: i32,
) {
    let p = plugin(e);
    let n = frames.max(0) as usize;
    if p.kind == Kind::Synth {
        // Echo the note-ons of this block back to the host (MIDI out).
        let mut out = [VstMidiEvent::default(); 16];
        let mut list = VstEventsBuf::<16> {
            num_events: 0,
            reserved: 0,
            events: [std::ptr::null_mut(); 16],
        };
        let mut k = 0;
        for &(delta, data) in &p.events[..p.num_events] {
            if data[0] & 0xF0 == 0x90 && data[2] > 0 && k < 16 {
                out[k] = VstMidiEvent {
                    event_type: kVstMidiType,
                    byte_size: 32,
                    delta_frames: delta,
                    midi_data: [data[0], data[1], data[2], 0],
                    ..VstMidiEvent::default()
                };
                k += 1;
            }
        }
        if k > 0 {
            for (slot, ev) in list.events.iter_mut().zip(out.iter_mut()).take(k) {
                *slot = (ev as *mut VstMidiEvent).cast();
            }
            list.num_events = k as i32;
            p.call_host(
                audioMasterProcessEvents,
                0,
                0,
                (&mut list as *mut VstEventsBuf<16>).cast(),
                0.0,
            );
        }
        p.render_synth(outputs, n);
        return;
    }
    p.read_time();
    let g = p.gain();
    for c in 0..2 {
        // SAFETY: two channels of `n` samples each way.
        unsafe {
            let i = std::slice::from_raw_parts(*inputs.add(c), n);
            let o = std::slice::from_raw_parts_mut(*outputs.add(c), n);
            for (o, i) in o.iter_mut().zip(i) {
                *o = i * g;
            }
        }
    }
}

unsafe extern "C" fn process_double_replacing(
    e: *mut AEffect,
    inputs: *mut *mut f64,
    outputs: *mut *mut f64,
    frames: i32,
) {
    let p = plugin(e);
    p.read_time();
    let g = f64::from(p.gain());
    let n = frames.max(0) as usize;
    for c in 0..2 {
        // SAFETY: two channels of `n` samples each way.
        unsafe {
            let i = std::slice::from_raw_parts(*inputs.add(c), n);
            let o = std::slice::from_raw_parts_mut(*outputs.add(c), n);
            for (o, i) in o.iter_mut().zip(i) {
                *o = i * g;
            }
        }
    }
}

/// Crash or hang on demand, based on this library's own path (see the module docs).
fn misbehave() {
    #[cfg(unix)]
    {
        use std::ffi::{CStr, c_char};
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
        // SAFETY: plain libc query about an address of this library.
        let mut info: DlInfo = unsafe { std::mem::zeroed() };
        let found = unsafe { dladdr(VSTPluginMain as *const c_void, &mut info) } != 0;
        if !found || info.fname.is_null() {
            return;
        }
        let path = unsafe { CStr::from_ptr(info.fname) }.to_string_lossy();
        if path.contains("ether-crash") {
            std::process::abort();
        }
        if path.contains("ether-hang") {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
}

/// The VST 2.4 entry point.
///
/// # Safety
/// Called by a VST2 host with a valid `audioMasterCallback`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn VSTPluginMain(host: HostCallback) -> *mut AEffect {
    misbehave();
    // SAFETY: a host callback accepts a null effect for `audioMasterVersion`/`CurrentId`.
    let version = unsafe { host(std::ptr::null_mut(), audioMasterVersion, 0, 0, std::ptr::null_mut(), 0.0) };
    if version == 0 {
        return std::ptr::null_mut();
    }
    let kind = if is_shell_build() {
        let id = unsafe {
            host(std::ptr::null_mut(), audioMasterCurrentId, 0, 0, std::ptr::null_mut(), 0.0)
        } as i32;
        match id {
            SHELL_GAIN_UID => Kind::ShellGain,
            SYNTH_UID => Kind::Synth,
            _ => Kind::Shell,
        }
    } else {
        Kind::Gain
    };
    let (uid, ins, outs, params, flags, delay) = match kind {
        Kind::Gain => (
            GAIN_UID,
            2,
            2,
            3,
            effFlagsCanReplacing | effFlagsCanDoubleReplacing | effFlagsProgramChunks,
            32,
        ),
        Kind::ShellGain => (SHELL_GAIN_UID, 2, 2, 3, effFlagsCanDoubleReplacing, 0),
        Kind::Synth => (
            SYNTH_UID,
            0,
            2,
            1,
            effFlagsCanReplacing | effFlagsIsSynth | effFlagsHasEditor,
            0,
        ),
        Kind::Shell => (SHELL_UID, 0, 0, 0, 0, 0),
    };
    let plugin = Box::new(Plugin {
        effect: AEffect {
            magic: EFFECT_MAGIC,
            dispatcher: Some(dispatcher),
            process: None,
            set_parameter: Some(set_parameter),
            get_parameter: Some(get_parameter),
            num_programs: if kind == Kind::Shell { 0 } else { 2 },
            num_params: params,
            num_inputs: ins,
            num_outputs: outs,
            flags,
            resvd1: 0,
            resvd2: 0,
            initial_delay: delay,
            real_qualities: 0,
            off_qualities: 0,
            io_ratio: 1.0,
            object: std::ptr::null_mut(),
            user: std::ptr::null_mut(),
            unique_id: uid,
            version: 1203,
            process_replacing: (kind != Kind::ShellGain).then_some(process_replacing as ProcessProc),
            process_double_replacing: (kind != Kind::Synth)
                .then_some(process_double_replacing as ProcessDoubleProc),
            future: [0; 56],
        },
        host,
        kind,
        params: [0.5, 0.0, 0.0],
        program: 0,
        chunk: Vec::new(),
        shell_index: 0,
        rect: ERect {
            top: 0,
            left: 0,
            bottom: 100,
            right: 200,
        },
        events: [(0, [0; 3]); MAX_EVENTS],
        num_events: 0,
        level: 0.0,
    });
    Box::into_raw(plugin).cast()
}
