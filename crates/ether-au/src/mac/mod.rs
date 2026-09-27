//! macOS implementation: the AudioComponent registry, `AUAudioUnit` instantiation, and the
//! run-loop pump used while waiting for asynchronous completions.

mod editor;
mod node;
mod params;
mod plugin;

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ether_core::plugin::PluginError;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_audio_toolbox::{
    AUAudioUnit, AudioComponent, AudioComponentCopyName, AudioComponentDescription,
    AudioComponentFindNext, AudioComponentFlags, AudioComponentGetDescription,
    AudioComponentGetVersion, AudioComponentInstantiationOptions,
};
use objc2_core_foundation::{CFRetained, CFRunLoop, CFString, kCFRunLoopDefaultMode};
use objc2_foundation::NSError;

use crate::AuComponentId;

pub use plugin::AuPlugin;

/// Component types we host: effects, instruments, music effects, MIDI processors.
const TYPES: [&[u8; 4]; 4] = [b"aufx", b"aumu", b"aumf", b"aumi"];

/// How long an asynchronous instantiation / view-controller request may take.
pub(crate) const ASYNC_TIMEOUT: Duration = Duration::from_secs(20);

fn desc_of(id: &AuComponentId) -> AudioComponentDescription {
    let (t, s, m) = id.os_types();
    AudioComponentDescription {
        componentType: t,
        componentSubType: s,
        componentManufacturer: m,
        componentFlags: 0,
        componentFlagsMask: 0,
    }
}

fn description(comp: AudioComponent) -> Option<AudioComponentDescription> {
    let mut d = AudioComponentDescription {
        componentType: 0,
        componentSubType: 0,
        componentManufacturer: 0,
        componentFlags: 0,
        componentFlagsMask: 0,
    };
    // SAFETY: `comp` came from AudioComponentFindNext; `d` is a valid out pointer.
    let status = unsafe { AudioComponentGetDescription(comp, NonNull::from(&mut d)) };
    (status == 0).then_some(d)
}

/// Every registered component of the hosted types (no plugin code runs).
pub(crate) fn list_registry() -> Vec<AuComponentId> {
    let mut out = Vec::new();
    for ty in TYPES {
        let mut query = AudioComponentDescription {
            componentType: u32::from_be_bytes(*ty),
            componentSubType: 0,
            componentManufacturer: 0,
            componentFlags: 0,
            componentFlagsMask: 0,
        };
        let mut comp: AudioComponent = std::ptr::null_mut();
        loop {
            // SAFETY: `comp` is null or the previous result; `query` is a valid pointer.
            comp = unsafe { AudioComponentFindNext(comp, NonNull::from(&mut query)) };
            if comp.is_null() {
                break;
            }
            if let Some(d) = description(comp) {
                out.push(AuComponentId::from_os_types(
                    d.componentType,
                    d.componentSubType,
                    d.componentManufacturer,
                ));
            }
        }
    }
    out.sort_by_key(|id| id.to_string());
    out.dedup();
    out
}

/// The registered component with exactly this id.
fn find(id: &AuComponentId) -> Option<AudioComponent> {
    let mut d = desc_of(id);
    // SAFETY: valid description pointer; null starts the search.
    let comp = unsafe { AudioComponentFindNext(std::ptr::null_mut(), NonNull::from(&mut d)) };
    (!comp.is_null()).then_some(comp)
}

/// `(name, vendor)` from the component name (`"Vendor: Name"` by convention).
pub(crate) fn split_name(full: &str) -> (String, String) {
    match full.split_once(':') {
        Some((vendor, name)) if !name.trim().is_empty() => {
            (name.trim().to_owned(), vendor.trim().to_owned())
        }
        _ => (full.trim().to_owned(), String::new()),
    }
}

/// `0xMMMMmmDD` → `"M.m.D"`.
pub(crate) fn version_string(v: u32) -> String {
    format!("{}.{}.{}", v >> 16, (v >> 8) & 0xFF, v & 0xFF)
}

struct ComponentInfo {
    name: String,
    vendor: String,
    version: String,
    flags: AudioComponentFlags,
}

fn info(comp: AudioComponent, id: &AuComponentId) -> ComponentInfo {
    let mut raw: *const CFString = std::ptr::null();
    // SAFETY: valid component and out pointer; on success we own the returned string.
    let full = if unsafe { AudioComponentCopyName(comp, NonNull::from(&mut raw)) } == 0 {
        NonNull::new(raw as *mut CFString)
            // SAFETY: "Copy" rule: +1 reference, released by CFRetained.
            .map(|p| unsafe { CFRetained::from_raw(p) }.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let (mut name, vendor) = split_name(&full);
    if name.is_empty() {
        name = id.to_string();
    }
    let mut v = 0u32;
    // SAFETY: valid component and out pointer.
    let version = if unsafe { AudioComponentGetVersion(comp, NonNull::from(&mut v)) } == 0 {
        version_string(v)
    } else {
        String::new()
    };
    let flags = description(comp)
        .map(|d| AudioComponentFlags(d.componentFlags))
        .unwrap_or(AudioComponentFlags(0));
    ComponentInfo {
        name,
        vendor,
        version,
        flags,
    }
}

/// Describe one component (scanner process). Reads the registry only: no instantiation.
pub(crate) fn scan(id: &AuComponentId) -> Result<PluginDescriptor, PluginError> {
    let comp = find(id).ok_or_else(|| PluginError::NotFound(id.to_string()))?;
    let info = info(comp, id);
    let ty = String::from_utf8_lossy(&id.component_type).into_owned();
    Ok(PluginDescriptor {
        format: PluginFormat::Au,
        id: id.to_string(),
        name: info.name,
        vendor: info.vendor,
        version: info.version,
        description: String::new(),
        features: vec![ty],
        category: id.category(),
        path: id.to_string(),
    })
}

/// Wrapper to move ObjC objects across the completion-handler boundary.
struct SendBox<T>(T);
// SAFETY: only used to hand a value from the completion handler to the waiting thread,
// which then owns it exclusively.
unsafe impl<T> Send for SendBox<T> {}

/// Pump the current thread's run loop until `slot` is filled or `timeout` elapses.
///
/// Completions of asynchronous AU calls may be delivered on the main run loop; blocking the
/// plugin main thread on a channel would then deadlock, so we run the loop while waiting.
pub(crate) fn pump_until<T>(slot: &Mutex<Option<T>>, timeout: Duration) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = slot.lock().ok().and_then(|mut s| s.take()) {
            return Some(v);
        }
        if start.elapsed() > timeout {
            return None;
        }
        // SAFETY: reading an immutable framework constant.
        let mode = unsafe { kCFRunLoopDefaultMode };
        CFRunLoop::run_in_mode(mode, 0.01, true);
    }
}

pub(crate) fn run_loop_for(d: Duration) {
    // SAFETY: reading an immutable framework constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, d.as_secs_f64(), true);
}

fn ns_error(e: &NSError) -> String {
    e.localizedDescription().to_string()
}

/// Instantiate `id` as an `AUAudioUnit` on the calling (plugin main) thread.
///
/// v2 components that don't require async instantiation are created synchronously
/// (in-process, via Apple's v2 bridge). v3 extensions and components flagged
/// `RequiresAsyncInstantiation` use `instantiateWithComponentDescription:options:
/// completionHandler:` while this thread's run loop is pumped ([`pump_until`]).
pub(crate) fn instantiate(
    id: &AuComponentId,
) -> Result<(Retained<AUAudioUnit>, String, String), PluginError> {
    let comp = find(id).ok_or_else(|| PluginError::NotFound(id.to_string()))?;
    let info = info(comp, id);
    let desc = desc_of(id);
    let is_async = info
        .flags
        .contains(AudioComponentFlags::RequiresAsyncInstantiation)
        || info.flags.contains(AudioComponentFlags::IsV3AudioUnit);
    let au = if !is_async {
        // SAFETY: plain initializer with a valid description.
        unsafe {
            AUAudioUnit::initWithComponentDescription_options_error(
                AUAudioUnit::alloc(),
                desc,
                AudioComponentInstantiationOptions(0),
            )
        }
        .map_err(|e| PluginError::Load(ns_error(&e)))?
    } else {
        type Done = Result<SendBox<Retained<AUAudioUnit>>, String>;
        let slot: Arc<Mutex<Option<Done>>> = Arc::new(Mutex::new(None));
        let tx = slot.clone();
        let handler = block2::RcBlock::new(move |au: *mut AUAudioUnit, err: *mut NSError| {
            // SAFETY: the handler receives +0 references; retain what we keep.
            let result = match unsafe { Retained::retain(au) } {
                Some(au) => Ok(SendBox(au)),
                None => Err(unsafe { err.as_ref() }
                    .map(ns_error)
                    .unwrap_or_else(|| "instantiation failed".into())),
            };
            if let Ok(mut s) = tx.lock() {
                *s = Some(result);
            }
        });
        // SAFETY: valid description; the block is copied by the callee.
        unsafe {
            AUAudioUnit::instantiateWithComponentDescription_options_completionHandler(
                desc,
                AudioComponentInstantiationOptions(0),
                &handler,
            );
        }
        match pump_until(&slot, ASYNC_TIMEOUT) {
            Some(Ok(au)) => au.0,
            Some(Err(e)) => return Err(PluginError::Load(e)),
            None => {
                return Err(PluginError::Load(
                    "timed out waiting for the Audio Unit to instantiate".into(),
                ));
            }
        }
    };
    Ok((au, info.name, info.vendor))
}
