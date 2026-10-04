//! The VST 2.4 plugin ABI, as minimal hand-written Rust bindings.
//!
//! # Provenance (no Steinberg SDK)
//! These declarations were written from two GPL-licensed, clean-room reimplementations of the
//! VST 2.x headers, never from the Steinberg VST2 SDK (which is not open source and is not
//! vendored, downloaded or consulted):
//! - **FST** ("Free Studio Technologies"), `fst/fst.h`, Copyright © 2019 IOhannes m zmölnig,
//!   IEM, GPL-3.0-or-later (<https://git.iem.at/zmoelnig/FST>): struct layouts of `AEffect`,
//!   `VstEvents`, `ERect`, effect/host opcodes, flags and plug categories.
//! - **VeSTige** `aeffectx.h` (LMMS, Copyright © 2006 Javier Serrano Polo, GPL-2.0-or-later;
//!   also shipped by Ardour): the `audioMaster*` opcode numbers FST leaves open (13
//!   `IOChanged`, 15 `SizeWindow`, ...), `VstMidiEvent` and `VstTimeInfo` field order.
//!
//! Both are GPL-compatible with this crate (GPL-3.0-or-later); see `THIRD_PARTY_NOTICES.txt`.
//! Only what Ethereal uses is declared. Layouts are checked by the tests at the bottom
//! (`size_of` / field offsets against the headers, for 64- and 32-bit targets).
//!
//! # Conventions
//! - Every function pointer uses the C calling convention (`cdecl` on 32-bit Windows; the
//!   only convention elsewhere).
//! - `VstIntPtr` (pointer-sized) is `isize`; `VstInt32` is `i32`.

#![allow(non_upper_case_globals)]

use std::ffi::c_void;

/// `VstIntPtr`: pointer-sized signed integer (dispatcher return values and `value` args).
pub type VstIntPtr = isize;

/// `AEffect::magic`: `'VstP'` as a big-endian four-char code.
pub const EFFECT_MAGIC: i32 = fourcc(*b"VstP");

/// The VST version this host implements (`audioMasterVersion`, `effGetVstVersion`): 2.4.
pub const VST_VERSION: i32 = 2400;

/// A four-char code as VST2 packs it (`CCONST`): first char in the most significant byte.
pub const fn fourcc(c: [u8; 4]) -> i32 {
    i32::from_be_bytes(c)
}

/// `dispatcher(effect, opcode, index, value, ptr, opt)`.
pub type DispatcherProc =
    unsafe extern "C" fn(*mut AEffect, i32, i32, VstIntPtr, *mut c_void, f32) -> VstIntPtr;
/// `audioMasterCallback`: same signature as the dispatcher, plugin → host.
pub type HostCallback =
    unsafe extern "C" fn(*mut AEffect, i32, i32, VstIntPtr, *mut c_void, f32) -> VstIntPtr;
/// `process` (accumulating, deprecated) and `processReplacing`.
pub type ProcessProc = unsafe extern "C" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32);
/// `processDoubleReplacing`.
pub type ProcessDoubleProc = unsafe extern "C" fn(*mut AEffect, *mut *mut f64, *mut *mut f64, i32);
pub type SetParameterProc = unsafe extern "C" fn(*mut AEffect, i32, f32);
pub type GetParameterProc = unsafe extern "C" fn(*mut AEffect, i32) -> f32;
/// The plugin entry point (`VSTPluginMain`, `main_macho`, `main`).
pub type PluginMain = unsafe extern "C" fn(HostCallback) -> *mut AEffect;

/// The plugin instance, allocated and owned by the plugin.
#[repr(C)]
pub struct AEffect {
    /// [`EFFECT_MAGIC`].
    pub magic: i32,
    pub dispatcher: Option<DispatcherProc>,
    /// Deprecated accumulating process (adds into the outputs).
    pub process: Option<ProcessProc>,
    pub set_parameter: Option<SetParameterProc>,
    pub get_parameter: Option<GetParameterProc>,
    pub num_programs: i32,
    pub num_params: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    /// `effFlags*` bits.
    pub flags: i32,
    /// Reserved for the host (must be 0 from the plugin). Ethereal leaves it alone.
    pub resvd1: VstIntPtr,
    /// Reserved for the host: Ethereal stores its per-instance host state here (as JUCE
    /// does), so `audioMasterCallback` can find it.
    pub resvd2: VstIntPtr,
    /// Latency in samples (PDC).
    pub initial_delay: i32,
    pub real_qualities: i32,
    pub off_qualities: i32,
    pub io_ratio: f32,
    /// The plugin's own object pointer.
    pub object: *mut c_void,
    pub user: *mut c_void,
    /// Registered plugin id (four-char code by convention).
    pub unique_id: i32,
    pub version: i32,
    pub process_replacing: Option<ProcessProc>,
    pub process_double_replacing: Option<ProcessDoubleProc>,
    /// Padding to the full 2.4 struct size (the host never reads it; plugins allocate it).
    pub future: [u8; 56],
}

/// Editor rectangle (`effEditGetRect`), in pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ERect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

/// Common header of every event in [`VstEvents`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VstEvent {
    /// [`kVstMidiType`], [`kVstSysExType`].
    pub event_type: i32,
    /// `size_of` the concrete event (`VstMidiEvent`: 32).
    pub byte_size: i32,
    /// Sample offset into the current block (sample-accurate MIDI).
    pub delta_frames: i32,
    pub flags: i32,
}

/// A short MIDI message.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VstMidiEvent {
    pub event_type: i32,
    pub byte_size: i32,
    pub delta_frames: i32,
    /// [`kVstMidiEventIsRealtime`].
    pub flags: i32,
    pub note_length: i32,
    pub note_offset: i32,
    /// Status, data1, data2, unused.
    pub midi_data: [u8; 4],
    pub detune: i8,
    pub note_off_velocity: u8,
    pub reserved1: u8,
    pub reserved2: u8,
}

/// `VstEvents` header; the event pointer array follows it in memory (C flexible array,
/// declared `events[2]` in the original). Use [`VstEventsBuf`] to own one.
#[repr(C)]
pub struct VstEvents {
    pub num_events: i32,
    pub reserved: VstIntPtr,
    pub events: [*mut VstEvent; 2],
}

/// A `VstEvents` with room for `N` event pointers (pre-allocated by the host).
#[repr(C)]
pub struct VstEventsBuf<const N: usize> {
    pub num_events: i32,
    pub reserved: VstIntPtr,
    pub events: [*mut VstEvent; N],
}

/// Transport and timing (`audioMasterGetTime`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VstTimeInfo {
    /// Project position in samples.
    pub sample_pos: f64,
    pub sample_rate: f64,
    pub nano_seconds: f64,
    /// Position in quarter notes.
    pub ppq_pos: f64,
    /// BPM.
    pub tempo: f64,
    /// Quarter-note position of the last bar start.
    pub bar_start_pos: f64,
    pub cycle_start_pos: f64,
    pub cycle_end_pos: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub smpte_offset: i32,
    pub smpte_frame_rate: i32,
    pub samples_to_next_clock: i32,
    /// `kVst*` validity / transport flags.
    pub flags: i32,
}

// --- effect opcodes (host → plugin, `dispatcher`) -------------------------------------------
pub const effOpen: i32 = 0;
pub const effClose: i32 = 1;
pub const effSetProgram: i32 = 2;
pub const effGetProgram: i32 = 3;
pub const effGetProgramName: i32 = 5;
pub const effGetParamLabel: i32 = 6;
pub const effGetParamDisplay: i32 = 7;
pub const effGetParamName: i32 = 8;
pub const effSetSampleRate: i32 = 10;
pub const effSetBlockSize: i32 = 11;
pub const effMainsChanged: i32 = 12;
pub const effEditGetRect: i32 = 13;
pub const effEditOpen: i32 = 14;
pub const effEditClose: i32 = 15;
pub const effEditIdle: i32 = 19;
pub const effGetChunk: i32 = 23;
pub const effSetChunk: i32 = 24;
pub const effProcessEvents: i32 = 25;
pub const effCanBeAutomated: i32 = 26;
pub const effGetPlugCategory: i32 = 35;
pub const effGetEffectName: i32 = 45;
pub const effGetVendorString: i32 = 47;
pub const effGetProductString: i32 = 48;
pub const effGetVendorVersion: i32 = 49;
pub const effVendorSpecific: i32 = 50;
pub const effCanDo: i32 = 51;
pub const effGetTailSize: i32 = 52;
pub const effIdle: i32 = 53;
pub const effGetVstVersion: i32 = 58;
pub const effShellGetNextPlugin: i32 = 70;
pub const effStartProcess: i32 = 71;
pub const effStopProcess: i32 = 72;
pub const effSetProcessPrecision: i32 = 77;

// --- host opcodes (plugin → host, `audioMasterCallback`) ------------------------------------
pub const audioMasterAutomate: i32 = 0;
pub const audioMasterVersion: i32 = 1;
pub const audioMasterCurrentId: i32 = 2;
pub const audioMasterIdle: i32 = 3;
pub const audioMasterWantMidi: i32 = 6;
pub const audioMasterGetTime: i32 = 7;
pub const audioMasterProcessEvents: i32 = 8;
pub const audioMasterTempoAt: i32 = 10;
pub const audioMasterIOChanged: i32 = 13;
pub const audioMasterNeedIdle: i32 = 14;
pub const audioMasterSizeWindow: i32 = 15;
pub const audioMasterGetSampleRate: i32 = 16;
pub const audioMasterGetBlockSize: i32 = 17;
pub const audioMasterGetInputLatency: i32 = 18;
pub const audioMasterGetOutputLatency: i32 = 19;
pub const audioMasterGetCurrentProcessLevel: i32 = 23;
pub const audioMasterGetAutomationState: i32 = 24;
pub const audioMasterGetVendorString: i32 = 32;
pub const audioMasterGetProductString: i32 = 33;
pub const audioMasterGetVendorVersion: i32 = 34;
pub const audioMasterCanDo: i32 = 37;
pub const audioMasterGetLanguage: i32 = 38;
pub const audioMasterGetDirectory: i32 = 41;
pub const audioMasterUpdateDisplay: i32 = 42;
pub const audioMasterBeginEdit: i32 = 43;
pub const audioMasterEndEdit: i32 = 44;

// --- AEffect::flags -------------------------------------------------------------------------
pub const effFlagsHasEditor: i32 = 1 << 0;
pub const effFlagsCanReplacing: i32 = 1 << 4;
pub const effFlagsProgramChunks: i32 = 1 << 5;
pub const effFlagsIsSynth: i32 = 1 << 8;
pub const effFlagsNoSoundInStop: i32 = 1 << 9;
pub const effFlagsCanDoubleReplacing: i32 = 1 << 12;

// --- effGetPlugCategory ---------------------------------------------------------------------
pub const kPlugCategUnknown: isize = 0;
pub const kPlugCategEffect: isize = 1;
pub const kPlugCategSynth: isize = 2;
pub const kPlugCategAnalysis: isize = 3;
pub const kPlugCategMastering: isize = 4;
pub const kPlugCategSpacializer: isize = 5;
pub const kPlugCategRoomFx: isize = 6;
pub const kPlugSurroundFx: isize = 7;
pub const kPlugCategRestoration: isize = 8;
pub const kPlugCategOfflineProcess: isize = 9;
pub const kPlugCategShell: isize = 10;
pub const kPlugCategGenerator: isize = 11;

// --- events ---------------------------------------------------------------------------------
pub const kVstMidiType: i32 = 1;
pub const kVstSysExType: i32 = 6;
pub const kVstMidiEventIsRealtime: i32 = 1;

// --- VstTimeInfo::flags ---------------------------------------------------------------------
pub const kVstTransportChanged: i32 = 1 << 0;
pub const kVstTransportPlaying: i32 = 1 << 1;
pub const kVstTransportCycleActive: i32 = 1 << 2;
pub const kVstTransportRecording: i32 = 1 << 3;
pub const kVstNanosValid: i32 = 1 << 8;
pub const kVstPpqPosValid: i32 = 1 << 9;
pub const kVstTempoValid: i32 = 1 << 10;
pub const kVstBarsValid: i32 = 1 << 11;
pub const kVstCyclePosValid: i32 = 1 << 12;
pub const kVstTimeSigValid: i32 = 1 << 13;

// --- misc -----------------------------------------------------------------------------------
pub const kVstProcessPrecision32: isize = 0;
pub const kVstProcessPrecision64: isize = 1;
pub const kVstProcessLevelUser: isize = 1;
pub const kVstProcessLevelRealtime: isize = 2;
pub const kVstLangEnglish: isize = 1;
/// String buffer size Ethereal passes to every string opcode. The nominal limits (FST:
/// 8 bytes for param strings, 25 for program names, 64/128 for names and vendor strings) are
/// routinely exceeded by plugins, so the host always over-allocates.
pub const STRING_BUF: usize = 256;

/// Read a NUL-terminated C string out of a fixed buffer (lossy UTF-8, trimmed).
pub fn read_cstr(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use std::mem::{offset_of, size_of};

    use super::*;

    /// Offsets of the 2.4 `AEffect` per the FST/VeSTige headers (`uniqueID` @112 on 64-bit,
    /// @0x48 on 32-bit; `processReplacing` @0x50 on 32-bit).
    #[test]
    fn aeffect_layout() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(offset_of!(AEffect, dispatcher), 8);
            assert_eq!(offset_of!(AEffect, process), 16);
            assert_eq!(offset_of!(AEffect, set_parameter), 24);
            assert_eq!(offset_of!(AEffect, get_parameter), 32);
            assert_eq!(offset_of!(AEffect, num_programs), 40);
            assert_eq!(offset_of!(AEffect, num_params), 44);
            assert_eq!(offset_of!(AEffect, num_inputs), 48);
            assert_eq!(offset_of!(AEffect, num_outputs), 52);
            assert_eq!(offset_of!(AEffect, flags), 56);
            assert_eq!(offset_of!(AEffect, resvd1), 64);
            assert_eq!(offset_of!(AEffect, resvd2), 72);
            assert_eq!(offset_of!(AEffect, initial_delay), 80);
            assert_eq!(offset_of!(AEffect, io_ratio), 92);
            assert_eq!(offset_of!(AEffect, object), 96);
            assert_eq!(offset_of!(AEffect, user), 104);
            assert_eq!(offset_of!(AEffect, unique_id), 112);
            assert_eq!(offset_of!(AEffect, version), 116);
            assert_eq!(offset_of!(AEffect, process_replacing), 120);
            assert_eq!(offset_of!(AEffect, process_double_replacing), 128);
            assert_eq!(size_of::<AEffect>(), 192);
        } else {
            assert_eq!(offset_of!(AEffect, dispatcher), 4);
            assert_eq!(offset_of!(AEffect, num_programs), 0x14);
            assert_eq!(offset_of!(AEffect, flags), 0x24);
            assert_eq!(offset_of!(AEffect, initial_delay), 0x30);
            assert_eq!(offset_of!(AEffect, object), 0x40);
            assert_eq!(offset_of!(AEffect, unique_id), 0x48);
            assert_eq!(offset_of!(AEffect, process_replacing), 0x50);
        }
    }

    #[test]
    fn event_layouts() {
        assert_eq!(size_of::<VstEvent>(), 16);
        assert_eq!(size_of::<VstMidiEvent>(), 32);
        assert_eq!(offset_of!(VstMidiEvent, delta_frames), 8);
        assert_eq!(offset_of!(VstMidiEvent, midi_data), 0x18);
        assert_eq!(offset_of!(VstMidiEvent, detune), 0x1c);
        assert_eq!(offset_of!(VstMidiEvent, note_off_velocity), 0x1d);
        let ptr = size_of::<usize>();
        // `numEvents` then a pointer-sized reserved field: the array starts at 2 words.
        assert_eq!(offset_of!(VstEvents, events), 2 * ptr);
        assert_eq!(offset_of!(VstEventsBuf<7>, events), 2 * ptr);
        assert_eq!(size_of::<VstEventsBuf<4>>(), 6 * ptr);
    }

    #[test]
    fn time_info_and_rect_layouts() {
        assert_eq!(offset_of!(VstTimeInfo, ppq_pos), 0x18);
        assert_eq!(offset_of!(VstTimeInfo, cycle_end_pos), 0x38);
        assert_eq!(offset_of!(VstTimeInfo, time_sig_numerator), 0x40);
        assert_eq!(offset_of!(VstTimeInfo, time_sig_denominator), 0x44);
        assert_eq!(offset_of!(VstTimeInfo, samples_to_next_clock), 0x50);
        assert_eq!(offset_of!(VstTimeInfo, flags), 0x54);
        assert_eq!(size_of::<VstTimeInfo>(), 88);
        assert_eq!(size_of::<ERect>(), 8);
        assert_eq!(offset_of!(ERect, left), 2);
        assert_eq!(offset_of!(ERect, right), 6);
    }

    #[test]
    fn constants() {
        assert_eq!(EFFECT_MAGIC, 0x5673_7450);
        assert_eq!(fourcc(*b"NvEf"), 1_316_373_862);
        assert_eq!(effFlagsCanDoubleReplacing, 4096);
        assert_eq!(read_cstr(b"Gain\0junk"), "Gain");
        assert_eq!(read_cstr(b" dB "), "dB");
    }
}
