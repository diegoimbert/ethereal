//! A tiny WebCLAP for the web host's tests (`apps/web/src/engine/webclap/*.test.ts`, the
//! e2e): the CLAP ABI written out by hand for wasm32, no dependencies. Three plugins:
//!
//! - `ether.test.gain`: stereo gain (param 1 `gain`, 0..2, sample-accurate), param 2
//!   `mode` (stepped 0..2, labels), state (both values), `SLEEP` on silent input, and a web
//!   UI (`clap.webview/3`, served by `get_resource`): the UI's `set:<gain>` message sets
//!   the gain as a plugin-side edit (gesture + param event), and the plugin answers
//!   `gain:<value>`.
//! - `ether.test.delay`: the gain plus a 32-sample delay it reports as latency.
//! - `ether.test.synth`: an instrument (one CLAP note port) that outputs the held note's
//!   velocity as DC on both channels, from the note-on's sample.
//!
//! Every host callback the plugins use is checked: they log through `clap.log` and ask
//! `clap.thread-check` where they are.
#![cfg(target_arch = "wasm32")]
#![allow(non_camel_case_types, clippy::missing_safety_doc)]

use std::alloc::{Layout, alloc_zeroed};
use std::ffi::{CStr, c_char, c_void};
use std::ptr::{null, null_mut};

// ─── CLAP ABI (the subset used) ──────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_version {
    major: u32,
    minor: u32,
    revision: u32,
}
const VERSION: clap_version = clap_version {
    major: 1,
    minor: 2,
    revision: 2,
};

#[repr(C)]
pub struct clap_plugin_entry {
    version: clap_version,
    init: unsafe extern "C" fn(*const c_char) -> bool,
    deinit: unsafe extern "C" fn(),
    get_factory: unsafe extern "C" fn(*const c_char) -> *const c_void,
}

#[repr(C)]
pub struct clap_host {
    version: clap_version,
    host_data: *mut c_void,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    host_version: *const c_char,
    get_extension: unsafe extern "C" fn(*const clap_host, *const c_char) -> *const c_void,
    request_restart: unsafe extern "C" fn(*const clap_host),
    request_process: unsafe extern "C" fn(*const clap_host),
    request_callback: unsafe extern "C" fn(*const clap_host),
}

#[repr(C)]
pub struct clap_plugin_descriptor {
    version: clap_version,
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    manual_url: *const c_char,
    support_url: *const c_char,
    plugin_version: *const c_char,
    description: *const c_char,
    features: *const *const c_char,
}

#[repr(C)]
pub struct clap_plugin {
    desc: *const clap_plugin_descriptor,
    plugin_data: *mut c_void,
    init: unsafe extern "C" fn(*const clap_plugin) -> bool,
    destroy: unsafe extern "C" fn(*const clap_plugin),
    activate: unsafe extern "C" fn(*const clap_plugin, f64, u32, u32) -> bool,
    deactivate: unsafe extern "C" fn(*const clap_plugin),
    start_processing: unsafe extern "C" fn(*const clap_plugin) -> bool,
    stop_processing: unsafe extern "C" fn(*const clap_plugin),
    reset: unsafe extern "C" fn(*const clap_plugin),
    process: unsafe extern "C" fn(*const clap_plugin, *const clap_process) -> i32,
    get_extension: unsafe extern "C" fn(*const clap_plugin, *const c_char) -> *const c_void,
    on_main_thread: unsafe extern "C" fn(*const clap_plugin),
}

#[repr(C)]
pub struct clap_plugin_factory {
    get_plugin_count: unsafe extern "C" fn(*const clap_plugin_factory) -> u32,
    get_plugin_descriptor:
        unsafe extern "C" fn(*const clap_plugin_factory, u32) -> *const clap_plugin_descriptor,
    create_plugin: unsafe extern "C" fn(
        *const clap_plugin_factory,
        *const clap_host,
        *const c_char,
    ) -> *const clap_plugin,
}

#[repr(C)]
pub struct clap_audio_buffer {
    data32: *mut *mut f32,
    data64: *mut *mut f64,
    channel_count: u32,
    latency: u32,
    constant_mask: u64,
}

#[repr(C)]
pub struct clap_process {
    steady_time: i64,
    frames_count: u32,
    transport: *const c_void,
    audio_inputs: *const clap_audio_buffer,
    audio_outputs: *mut clap_audio_buffer,
    audio_inputs_count: u32,
    audio_outputs_count: u32,
    in_events: *const clap_input_events,
    out_events: *const clap_output_events,
}

#[repr(C)]
pub struct clap_input_events {
    ctx: *mut c_void,
    size: unsafe extern "C" fn(*const clap_input_events) -> u32,
    get: unsafe extern "C" fn(*const clap_input_events, u32) -> *const clap_event_header,
}

#[repr(C)]
pub struct clap_output_events {
    ctx: *mut c_void,
    try_push: unsafe extern "C" fn(*const clap_output_events, *const clap_event_header) -> bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_event_header {
    size: u32,
    time: u32,
    space_id: u16,
    type_: u16,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_event_note {
    header: clap_event_header,
    note_id: i32,
    port_index: i16,
    channel: i16,
    key: i16,
    velocity: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_event_param_value {
    header: clap_event_header,
    param_id: u32,
    cookie: *mut c_void,
    note_id: i32,
    port_index: i16,
    channel: i16,
    key: i16,
    value: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_event_param_gesture {
    header: clap_event_header,
    param_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_event_midi {
    header: clap_event_header,
    port_index: u16,
    data: [u8; 3],
}

const EVENT_NOTE_ON: u16 = 0;
const EVENT_NOTE_OFF: u16 = 1;
const EVENT_NOTE_CHOKE: u16 = 2;
const EVENT_PARAM_VALUE: u16 = 5;
const EVENT_PARAM_GESTURE_BEGIN: u16 = 7;
const EVENT_PARAM_GESTURE_END: u16 = 8;
const EVENT_MIDI: u16 = 10;

const PROCESS_CONTINUE: i32 = 1;
const PROCESS_SLEEP: i32 = 4;

#[repr(C)]
pub struct clap_param_info {
    id: u32,
    flags: u32,
    cookie: *mut c_void,
    name: [c_char; 256],
    module: [c_char; 1024],
    min_value: f64,
    max_value: f64,
    default_value: f64,
}
const PARAM_IS_STEPPED: u32 = 1 << 0;
const PARAM_IS_AUTOMATABLE: u32 = 1 << 5;

#[repr(C)]
pub struct clap_plugin_params {
    count: unsafe extern "C" fn(*const clap_plugin) -> u32,
    get_info: unsafe extern "C" fn(*const clap_plugin, u32, *mut clap_param_info) -> bool,
    get_value: unsafe extern "C" fn(*const clap_plugin, u32, *mut f64) -> bool,
    value_to_text: unsafe extern "C" fn(*const clap_plugin, u32, f64, *mut c_char, u32) -> bool,
    text_to_value: unsafe extern "C" fn(*const clap_plugin, u32, *const c_char, *mut f64) -> bool,
    flush: unsafe extern "C" fn(
        *const clap_plugin,
        *const clap_input_events,
        *const clap_output_events,
    ),
}

#[repr(C)]
pub struct clap_host_params {
    rescan: unsafe extern "C" fn(*const clap_host, u32),
    clear: unsafe extern "C" fn(*const clap_host, u32, u32),
    request_flush: unsafe extern "C" fn(*const clap_host),
}

#[repr(C)]
pub struct clap_istream {
    ctx: *mut c_void,
    read: unsafe extern "C" fn(*const clap_istream, *mut c_void, u64) -> i64,
}
#[repr(C)]
pub struct clap_ostream {
    ctx: *mut c_void,
    write: unsafe extern "C" fn(*const clap_ostream, *const c_void, u64) -> i64,
}
#[repr(C)]
pub struct clap_plugin_state {
    save: unsafe extern "C" fn(*const clap_plugin, *const clap_ostream) -> bool,
    load: unsafe extern "C" fn(*const clap_plugin, *const clap_istream) -> bool,
}
#[repr(C)]
pub struct clap_host_state {
    mark_dirty: unsafe extern "C" fn(*const clap_host),
}

#[repr(C)]
pub struct clap_audio_port_info {
    id: u32,
    name: [c_char; 256],
    flags: u32,
    channel_count: u32,
    port_type: *const c_char,
    in_place_pair: u32,
}
#[repr(C)]
pub struct clap_plugin_audio_ports {
    count: unsafe extern "C" fn(*const clap_plugin, bool) -> u32,
    get: unsafe extern "C" fn(*const clap_plugin, u32, bool, *mut clap_audio_port_info) -> bool,
}

#[repr(C)]
pub struct clap_note_port_info {
    id: u32,
    supported_dialects: u32,
    preferred_dialect: u32,
    name: [c_char; 256],
}
#[repr(C)]
pub struct clap_plugin_note_ports {
    count: unsafe extern "C" fn(*const clap_plugin, bool) -> u32,
    get: unsafe extern "C" fn(*const clap_plugin, u32, bool, *mut clap_note_port_info) -> bool,
}

#[repr(C)]
pub struct clap_plugin_latency {
    get: unsafe extern "C" fn(*const clap_plugin) -> u32,
}

#[repr(C)]
pub struct clap_plugin_webview {
    get_uri: unsafe extern "C" fn(*const clap_plugin, *mut c_char, u32) -> i32,
    get_resource: unsafe extern "C" fn(
        *const clap_plugin,
        *const c_char,
        *mut c_char,
        u32,
        *const clap_ostream,
    ) -> bool,
    receive: unsafe extern "C" fn(*const clap_plugin, *const c_void, u32) -> bool,
}
#[repr(C)]
pub struct clap_host_webview {
    send: unsafe extern "C" fn(*const clap_host, *const c_void, u32) -> bool,
}

#[repr(C)]
pub struct clap_host_log {
    log: unsafe extern "C" fn(*const clap_host, i32, *const c_char),
}
#[repr(C)]
pub struct clap_host_thread_check {
    is_main_thread: unsafe extern "C" fn(*const clap_host) -> bool,
    is_audio_thread: unsafe extern "C" fn(*const clap_host) -> bool,
}

// ─── Statics ─────────────────────────────────────────────────────────────────────────────

/// Pointers in statics (read-only after start).
struct Sync<T>(T);
unsafe impl<T> std::marker::Sync for Sync<T> {}

macro_rules! cstr {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

static EFFECT_FEATURES: Sync<[*const c_char; 3]> =
    Sync([cstr!("audio-effect"), cstr!("stereo"), null()]);
static SYNTH_FEATURES: Sync<[*const c_char; 3]> =
    Sync([cstr!("instrument"), cstr!("synthesizer"), null()]);

const fn desc(
    id: *const c_char,
    name: *const c_char,
    features: *const *const c_char,
) -> clap_plugin_descriptor {
    clap_plugin_descriptor {
        version: VERSION,
        id,
        name,
        vendor: cstr!("Ethereal"),
        url: cstr!(""),
        manual_url: cstr!(""),
        support_url: cstr!(""),
        plugin_version: cstr!("1.0.0"),
        description: cstr!("test plugin"),
        features,
    }
}

static DESCRIPTORS: Sync<[clap_plugin_descriptor; 3]> = Sync([
    desc(
        cstr!("ether.test.gain"),
        cstr!("Test Gain"),
        &EFFECT_FEATURES.0 as *const _ as *const *const c_char,
    ),
    desc(
        cstr!("ether.test.delay"),
        cstr!("Test Delay"),
        &EFFECT_FEATURES.0 as *const _ as *const *const c_char,
    ),
    desc(
        cstr!("ether.test.synth"),
        cstr!("Test Synth"),
        &SYNTH_FEATURES.0 as *const _ as *const *const c_char,
    ),
]);

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Gain,
    Delay,
    Synth,
}

const DELAY_SAMPLES: u32 = 32;
const P_GAIN: u32 = 1;
const P_MODE: u32 = 2;

// ─── Allocation (the host reserves its structures through `malloc`) ──────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut u8 {
    // Never freed (the host keeps what it reserves for the instance's lifetime).
    unsafe { alloc_zeroed(Layout::from_size_align(size.max(1), 16).expect("layout")) }
}

// ─── Entry and factory ───────────────────────────────────────────────────────────────────

static mut INIT_PATH_OK: bool = false;

unsafe extern "C" fn entry_init(path: *const c_char) -> bool {
    unsafe { INIT_PATH_OK = !path.is_null() };
    true
}
unsafe extern "C" fn entry_deinit() {}
unsafe extern "C" fn entry_get_factory(id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) }.to_bytes() == b"clap.plugin-factory" {
        &FACTORY as *const clap_plugin_factory as *const c_void
    } else {
        null()
    }
}

#[unsafe(no_mangle)]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    version: VERSION,
    init: entry_init,
    deinit: entry_deinit,
    get_factory: entry_get_factory,
};

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: factory_count,
    get_plugin_descriptor: factory_descriptor,
    create_plugin: factory_create,
};

unsafe extern "C" fn factory_count(_: *const clap_plugin_factory) -> u32 {
    DESCRIPTORS.0.len() as u32
}
unsafe extern "C" fn factory_descriptor(
    _: *const clap_plugin_factory,
    i: u32,
) -> *const clap_plugin_descriptor {
    DESCRIPTORS.0.get(i as usize).map_or(null(), |d| d as *const _)
}

struct Plugin {
    clap: clap_plugin,
    host: *const clap_host,
    kind: Kind,
    gain: f64,
    mode: f64,
    sample_rate: f64,
    delay: Vec<[f32; 2]>,
    pos: usize,
    /// The held note (synth): its velocity.
    note: Option<(i16, f64)>,
    /// The UI's gain, to report as a plugin edit at the next flush/process.
    ui_gain: Option<f64>,
    host_params: *const clap_host_params,
    host_state: *const clap_host_state,
    host_webview: *const clap_host_webview,
    host_log: *const clap_host_log,
    host_thread: *const clap_host_thread_check,
}

unsafe fn plugin<'a>(p: *const clap_plugin) -> &'a mut Plugin {
    unsafe { &mut *((*p).plugin_data as *mut Plugin) }
}

unsafe extern "C" fn factory_create(
    _: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    let id = unsafe { CStr::from_ptr(id) }.to_bytes();
    let (index, kind) = match id {
        b"ether.test.gain" => (0, Kind::Gain),
        b"ether.test.delay" => (1, Kind::Delay),
        b"ether.test.synth" => (2, Kind::Synth),
        _ => return null(),
    };
    let p = Box::new(Plugin {
        clap: clap_plugin {
            desc: &DESCRIPTORS.0[index],
            plugin_data: null_mut(),
            init: plugin_init,
            destroy: plugin_destroy,
            activate: plugin_activate,
            deactivate: plugin_noop,
            start_processing: plugin_start,
            stop_processing: plugin_noop,
            reset: plugin_reset,
            process: plugin_process,
            get_extension: plugin_get_extension,
            on_main_thread: plugin_noop,
        },
        host,
        kind,
        gain: 1.0,
        mode: 0.0,
        sample_rate: 0.0,
        delay: Vec::new(),
        pos: 0,
        note: None,
        ui_gain: None,
        host_params: null(),
        host_state: null(),
        host_webview: null(),
        host_log: null(),
        host_thread: null(),
    });
    let raw = Box::into_raw(p);
    unsafe {
        (*raw).clap.plugin_data = raw as *mut c_void;
        &(*raw).clap
    }
}

unsafe fn host_ext<T>(host: *const clap_host, id: *const c_char) -> *const T {
    unsafe { ((*host).get_extension)(host, id) as *const T }
}

unsafe fn log(p: &Plugin, severity: i32, msg: *const c_char) {
    if !p.host_log.is_null() {
        unsafe { ((*p.host_log).log)(p.host, severity, msg) };
    }
}

unsafe fn on_main(p: &Plugin) -> bool {
    p.host_thread.is_null() || unsafe { ((*p.host_thread).is_main_thread)(p.host) }
}

unsafe extern "C" fn plugin_init(cp: *const clap_plugin) -> bool {
    let p = unsafe { plugin(cp) };
    unsafe {
        p.host_params = host_ext(p.host, cstr!("clap.params"));
        p.host_state = host_ext(p.host, cstr!("clap.state"));
        p.host_webview = host_ext(p.host, cstr!("clap.webview/3"));
        p.host_log = host_ext(p.host, cstr!("clap.log"));
        p.host_thread = host_ext(p.host, cstr!("clap.thread-check"));
        if !on_main(p) {
            log(p, 3, cstr!("init called off the main thread"));
        }
        if !INIT_PATH_OK {
            log(p, 3, cstr!("clap_entry.init got no path"));
        }
        log(p, 0, cstr!("ether test plugin ready"));
    }
    true
}

unsafe extern "C" fn plugin_destroy(cp: *const clap_plugin) {
    drop(unsafe { Box::from_raw((*cp).plugin_data as *mut Plugin) });
}

unsafe extern "C" fn plugin_noop(_: *const clap_plugin) {}

unsafe extern "C" fn plugin_activate(cp: *const clap_plugin, sr: f64, _min: u32, _max: u32) -> bool {
    let p = unsafe { plugin(cp) };
    p.sample_rate = sr;
    if p.kind == Kind::Delay {
        p.delay = vec![[0.0; 2]; DELAY_SAMPLES as usize];
        p.pos = 0;
    }
    true
}

unsafe extern "C" fn plugin_start(cp: *const clap_plugin) -> bool {
    let p = unsafe { plugin(cp) };
    if unsafe { on_main(p) } {
        unsafe { log(p, 3, cstr!("start_processing called on the main thread")) };
    }
    true
}

unsafe extern "C" fn plugin_reset(cp: *const clap_plugin) {
    let p = unsafe { plugin(cp) };
    p.delay.iter_mut().for_each(|s| *s = [0.0; 2]);
    p.note = None;
}

unsafe fn apply_event(p: &mut Plugin, e: *const clap_event_header) {
    let h = unsafe { *e };
    if h.space_id != 0 {
        return;
    }
    match h.type_ {
        EVENT_PARAM_VALUE => {
            let e = unsafe { *(e as *const clap_event_param_value) };
            match e.param_id {
                P_GAIN => p.gain = e.value.clamp(0.0, 2.0),
                P_MODE => p.mode = e.value.round().clamp(0.0, 2.0),
                _ => {}
            }
        }
        EVENT_NOTE_ON => {
            let e = unsafe { *(e as *const clap_event_note) };
            p.note = Some((e.key, e.velocity));
        }
        EVENT_NOTE_OFF | EVENT_NOTE_CHOKE => {
            let e = unsafe { *(e as *const clap_event_note) };
            if e.key < 0 || p.note.is_some_and(|(k, _)| k == e.key) {
                p.note = None;
            }
        }
        EVENT_MIDI => {
            let e = unsafe { *(e as *const clap_event_midi) };
            if e.data[0] & 0xF0 == 0xB0 && e.data[1] == 123 {
                p.note = None;
            }
        }
        _ => {}
    }
}

/// The UI's pending edit as plugin output events.
unsafe fn push_ui_edit(p: &mut Plugin, out: *const clap_output_events) {
    let Some(gain) = p.ui_gain.take() else {
        return;
    };
    p.gain = gain;
    let header = |size: usize, type_| clap_event_header {
        size: size as u32,
        time: 0,
        space_id: 0,
        type_,
        flags: 0,
    };
    let begin = clap_event_param_gesture {
        header: header(
            size_of::<clap_event_param_gesture>(),
            EVENT_PARAM_GESTURE_BEGIN,
        ),
        param_id: P_GAIN,
    };
    let value = clap_event_param_value {
        header: header(size_of::<clap_event_param_value>(), EVENT_PARAM_VALUE),
        param_id: P_GAIN,
        cookie: null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: gain,
    };
    let end = clap_event_param_gesture {
        header: header(
            size_of::<clap_event_param_gesture>(),
            EVENT_PARAM_GESTURE_END,
        ),
        param_id: P_GAIN,
    };
    unsafe {
        ((*out).try_push)(out, &begin.header);
        ((*out).try_push)(out, &value.header);
        ((*out).try_push)(out, &end.header);
    }
}

unsafe extern "C" fn plugin_process(cp: *const clap_plugin, pr: *const clap_process) -> i32 {
    let p = unsafe { plugin(cp) };
    let pr = unsafe { &*pr };
    unsafe { push_ui_edit(p, pr.out_events) };
    let frames = pr.frames_count as usize;
    let ins = unsafe { &*pr.in_events };
    let n_events = unsafe { (ins.size)(ins) };
    let mut next = 0;
    let out = unsafe { &*pr.audio_outputs };
    let input = (pr.audio_inputs_count > 0).then(|| unsafe { &*pr.audio_inputs });
    let mut silent = true;
    for i in 0..frames {
        while next < n_events {
            let e = unsafe { (ins.get)(ins, next) };
            if unsafe { (*e).time } as usize > i {
                break;
            }
            unsafe { apply_event(p, e) };
            next += 1;
        }
        for c in 0..out.channel_count.min(2) as usize {
            let x = match (p.kind, input) {
                (Kind::Synth, _) => p.note.map_or(0.0, |(_, v)| v as f32),
                (_, Some(b)) if (c as u32) < b.channel_count => unsafe {
                    *(*b.data32.add(c)).add(i) * p.gain as f32
                },
                _ => 0.0,
            };
            silent &= x == 0.0;
            let mut y = x;
            if p.kind == Kind::Delay {
                std::mem::swap(&mut p.delay[p.pos][c], &mut y);
            }
            unsafe { *(*out.data32.add(c)).add(i) = y };
        }
        if p.kind == Kind::Delay {
            p.pos = (p.pos + 1) % p.delay.len();
        }
    }
    while next < n_events {
        unsafe { apply_event(p, (ins.get)(ins, next)) };
        next += 1;
    }
    match p.kind {
        Kind::Delay => PROCESS_CONTINUE,
        Kind::Synth if p.note.is_some() => PROCESS_CONTINUE,
        _ if silent => PROCESS_SLEEP,
        _ => PROCESS_CONTINUE,
    }
}

// ─── Extensions ──────────────────────────────────────────────────────────────────────────

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: params_count,
    get_info: params_info,
    get_value: params_value,
    value_to_text: params_to_text,
    text_to_value: params_from_text,
    flush: params_flush,
};
static STATE: clap_plugin_state = clap_plugin_state {
    save: state_save,
    load: state_load,
};
static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: ports_count,
    get: ports_get,
};
static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: note_ports_count,
    get: note_ports_get,
};
static LATENCY: clap_plugin_latency = clap_plugin_latency { get: latency_get };
static WEBVIEW: clap_plugin_webview = clap_plugin_webview {
    get_uri: webview_uri,
    get_resource: webview_resource,
    receive: webview_receive,
};

unsafe extern "C" fn plugin_get_extension(cp: *const clap_plugin, id: *const c_char) -> *const c_void {
    let p = unsafe { plugin(cp) };
    let id = unsafe { CStr::from_ptr(id) }.to_bytes();
    let ext: *const c_void = match id {
        b"clap.params" => &PARAMS as *const _ as _,
        b"clap.state" => &STATE as *const _ as _,
        b"clap.audio-ports" => &AUDIO_PORTS as *const _ as _,
        b"clap.note-ports" if p.kind == Kind::Synth => &NOTE_PORTS as *const _ as _,
        b"clap.latency" if p.kind == Kind::Delay => &LATENCY as *const _ as _,
        b"clap.webview/3" if p.kind == Kind::Gain => &WEBVIEW as *const _ as _,
        _ => null(),
    };
    ext
}

unsafe extern "C" fn params_count(_: *const clap_plugin) -> u32 {
    2
}

fn write_str(dst: &mut [c_char], s: &[u8]) {
    let n = s.len().min(dst.len() - 1);
    for (d, &b) in dst.iter_mut().zip(&s[..n]) {
        *d = b as c_char;
    }
    dst[n] = 0;
}

unsafe extern "C" fn params_info(_: *const clap_plugin, i: u32, info: *mut clap_param_info) -> bool {
    let info = unsafe { &mut *info };
    let (id, flags, name, max, default) = match i {
        0 => (P_GAIN, PARAM_IS_AUTOMATABLE, &b"gain"[..], 2.0, 1.0),
        1 => (
            P_MODE,
            PARAM_IS_AUTOMATABLE | PARAM_IS_STEPPED,
            &b"mode"[..],
            2.0,
            0.0,
        ),
        _ => return false,
    };
    info.id = id;
    info.flags = flags;
    info.cookie = null_mut();
    write_str(&mut info.name, name);
    write_str(&mut info.module, b"main");
    info.min_value = 0.0;
    info.max_value = max;
    info.default_value = default;
    true
}

unsafe extern "C" fn params_value(cp: *const clap_plugin, id: u32, out: *mut f64) -> bool {
    let p = unsafe { plugin(cp) };
    let v = match id {
        P_GAIN => p.ui_gain.unwrap_or(p.gain),
        P_MODE => p.mode,
        _ => return false,
    };
    unsafe { *out = v };
    true
}

unsafe extern "C" fn params_to_text(
    _: *const clap_plugin,
    id: u32,
    v: f64,
    out: *mut c_char,
    cap: u32,
) -> bool {
    let text = match id {
        P_GAIN => format!("{:.2}x", v),
        P_MODE => ["soft", "medium", "hard"]
            .get(v.round() as usize)
            .map_or_else(|| "?".to_string(), |s| s.to_string()),
        _ => return false,
    };
    let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    write_str(dst, text.as_bytes());
    true
}

unsafe extern "C" fn params_from_text(
    _: *const clap_plugin,
    _: u32,
    _: *const c_char,
    _: *mut f64,
) -> bool {
    false
}

unsafe extern "C" fn params_flush(
    cp: *const clap_plugin,
    ins: *const clap_input_events,
    out: *const clap_output_events,
) {
    let p = unsafe { plugin(cp) };
    let ins = unsafe { &*ins };
    for i in 0..unsafe { (ins.size)(ins) } {
        unsafe { apply_event(p, (ins.get)(ins, i)) };
    }
    unsafe { push_ui_edit(p, out) };
}

unsafe extern "C" fn state_save(cp: *const clap_plugin, s: *const clap_ostream) -> bool {
    let p = unsafe { plugin(cp) };
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&p.gain.to_le_bytes());
    bytes[8..].copy_from_slice(&p.mode.to_le_bytes());
    let mut done = 0;
    while done < bytes.len() {
        let n = unsafe {
            ((*s).write)(
                s,
                bytes[done..].as_ptr() as *const c_void,
                (bytes.len() - done) as u64,
            )
        };
        if n <= 0 {
            return false;
        }
        done += n as usize;
    }
    true
}

unsafe extern "C" fn state_load(cp: *const clap_plugin, s: *const clap_istream) -> bool {
    let p = unsafe { plugin(cp) };
    let mut bytes = [0u8; 16];
    let mut done = 0;
    while done < bytes.len() {
        let n = unsafe {
            ((*s).read)(
                s,
                bytes[done..].as_mut_ptr() as *mut c_void,
                (bytes.len() - done) as u64,
            )
        };
        if n <= 0 {
            break;
        }
        done += n as usize;
    }
    if done != bytes.len() {
        return false;
    }
    p.gain = f64::from_le_bytes(bytes[..8].try_into().expect("8"));
    p.mode = f64::from_le_bytes(bytes[8..].try_into().expect("8"));
    true
}

unsafe extern "C" fn ports_count(cp: *const clap_plugin, is_input: bool) -> u32 {
    let p = unsafe { plugin(cp) };
    u32::from(!(is_input && p.kind == Kind::Synth))
}

unsafe extern "C" fn ports_get(
    cp: *const clap_plugin,
    i: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    let p = unsafe { plugin(cp) };
    if i != 0 || (is_input && p.kind == Kind::Synth) {
        return false;
    }
    let info = unsafe { &mut *info };
    info.id = 0;
    write_str(&mut info.name, if is_input { b"in" } else { b"out" });
    info.flags = 1; // CLAP_AUDIO_PORT_IS_MAIN
    info.channel_count = 2;
    info.port_type = cstr!("stereo");
    info.in_place_pair = u32::MAX;
    true
}

unsafe extern "C" fn note_ports_count(_: *const clap_plugin, is_input: bool) -> u32 {
    u32::from(is_input)
}

unsafe extern "C" fn note_ports_get(
    _: *const clap_plugin,
    i: u32,
    is_input: bool,
    info: *mut clap_note_port_info,
) -> bool {
    if i != 0 || !is_input {
        return false;
    }
    let info = unsafe { &mut *info };
    info.id = 0;
    info.supported_dialects = 1 | 2; // CLAP | MIDI
    info.preferred_dialect = 1;
    write_str(&mut info.name, b"notes");
    true
}

unsafe extern "C" fn latency_get(_: *const clap_plugin) -> u32 {
    DELAY_SAMPLES
}

/// The UI: served by `get_resource` (a relative URI: the host picks the scheme).
const UI_HTML: &str = r#"<!doctype html><html><body>
<p id="gain">gain ?</p><button id="half" onclick="send('set:0.5')">half</button>
<script>
function send(t) { parent.postMessage(new TextEncoder().encode(t).buffer, "*"); }
addEventListener("message", (e) => {
  if (!(e.data instanceof ArrayBuffer)) return;
  const t = new TextDecoder().decode(e.data);
  document.getElementById("gain").textContent = t.replace(":", " ");
  document.body.dataset.last = t;
});
send("ready");
</script></body></html>"#;

unsafe extern "C" fn webview_uri(_: *const clap_plugin, out: *mut c_char, cap: u32) -> i32 {
    let uri = b"/ui/index.html";
    if cap as usize > uri.len() {
        let dst = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
        write_str(dst, uri);
    }
    uri.len() as i32 + 1
}

unsafe extern "C" fn webview_resource(
    _: *const clap_plugin,
    path: *const c_char,
    mime: *mut c_char,
    cap: u32,
    s: *const clap_ostream,
) -> bool {
    if unsafe { CStr::from_ptr(path) }.to_bytes() != b"/ui/index.html" {
        return false;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(mime, cap as usize) };
    write_str(dst, b"text/html");
    let bytes = UI_HTML.as_bytes();
    let n = unsafe { ((*s).write)(s, bytes.as_ptr() as *const c_void, bytes.len() as u64) };
    n as usize == bytes.len()
}

unsafe extern "C" fn webview_receive(cp: *const clap_plugin, buf: *const c_void, size: u32) -> bool {
    let p = unsafe { plugin(cp) };
    if unsafe { !on_main(p) } {
        unsafe { log(p, 3, cstr!("webview receive off the main thread")) };
    }
    let msg = unsafe { std::slice::from_raw_parts(buf as *const u8, size as usize) };
    let reply = if let Some(v) = msg.strip_prefix(b"set:") {
        let Some(gain) = std::str::from_utf8(v).ok().and_then(|s| s.parse::<f64>().ok()) else {
            return false;
        };
        p.ui_gain = Some(gain.clamp(0.0, 2.0));
        unsafe {
            if !p.host_params.is_null() {
                ((*p.host_params).request_flush)(p.host);
            }
            if !p.host_state.is_null() {
                ((*p.host_state).mark_dirty)(p.host);
            }
        }
        format!("gain:{gain}")
    } else if msg == b"ready" {
        format!("gain:{}", p.gain)
    } else {
        return false;
    };
    if !p.host_webview.is_null() {
        unsafe {
            ((*p.host_webview).send)(p.host, reply.as_ptr() as *const c_void, reply.len() as u32)
        };
    }
    true
}
