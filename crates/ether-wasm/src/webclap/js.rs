//! The JS host glue as this realm's [`Guest`]: `WebclapHost` in
//! `apps/web/src/engine/webclap/host.ts`, handed to `WasmEngine::set_webclap_host` (the
//! Worklet) or `WasmController::set_webclap_host` (offline renders in the Worker).
//!
//! Every per-block call passes numbers only (no JS allocation): the block itself is read
//! by JS from this module's memory at the [`Exchange`]'s address.

use wasm_bindgen::prelude::*;

use super::guest::{Guest, GuestNote, GuestSpec};
use super::node::Exchange;

#[wasm_bindgen]
extern "C" {
    /// `WebclapHost` (host.ts).
    pub type JsWebclapHost;

    #[wasm_bindgen(method)]
    fn create(
        this: &JsWebclapHost,
        module: &str,
        plugin_id: &str,
        device: &str,
        sample_rate: f64,
        max_frames: u32,
        state: Option<Vec<u8>>,
    ) -> u32;
    #[wasm_bindgen(method)]
    fn process(this: &JsWebclapHost, handle: u32, exchange: u32) -> i32;
    /// 1 ready, 0 pending; < 0 failed (`error(handle)` says why).
    #[wasm_bindgen(method)]
    fn ready(this: &JsWebclapHost, handle: u32) -> i32;
    #[wasm_bindgen(method)]
    fn error(this: &JsWebclapHost, handle: u32) -> String;
    #[wasm_bindgen(method)]
    fn latency(this: &JsWebclapHost, handle: u32) -> u32;
    #[wasm_bindgen(method, js_name = takeWake)]
    fn take_wake(this: &JsWebclapHost, handle: u32) -> bool;
    #[wasm_bindgen(method)]
    fn reset(this: &JsWebclapHost, handle: u32);
    #[wasm_bindgen(method)]
    fn destroy(this: &JsWebclapHost, handle: u32);
    /// A `Uint8Array`, `null` (no state extension), or throws.
    #[wasm_bindgen(method, catch, js_name = saveState)]
    fn save_state(this: &JsWebclapHost, handle: u32) -> Result<JsValue, JsValue>;
    /// A `Float64Array` of `[id, value, …]`, or `null`.
    #[wasm_bindgen(method, js_name = paramValues)]
    fn param_values(this: &JsWebclapHost, handle: u32) -> JsValue;
    /// `undefined` when there is nothing, else an array of `[handle, kind, a, b, text]`.
    #[wasm_bindgen(method, js_name = takeNotifications)]
    fn take_notifications(this: &JsWebclapHost) -> JsValue;
}

/// Kinds of `takeNotifications` entries (host.ts `Note`).
mod note {
    pub const PARAM: u32 = 1;
    pub const BEGIN: u32 = 2;
    pub const END: u32 = 3;
    pub const DIRTY: u32 = 4;
    pub const PARAMS: u32 = 5;
    pub const CRASHED: u32 = 6;
}

pub struct JsGuest(pub JsWebclapHost);

impl Guest for JsGuest {
    fn create(&self, spec: &GuestSpec<'_>) -> u32 {
        self.0.create(
            spec.module,
            spec.plugin_id,
            spec.device,
            spec.sample_rate,
            spec.max_frames,
            spec.state.map(<[u8]>::to_vec),
        )
    }

    fn process(&self, handle: u32, exchange: &mut Exchange) -> i32 {
        self.0
            .process(handle, std::ptr::from_mut(exchange) as usize as u32)
    }

    fn ready(&self, handle: u32) -> Result<bool, String> {
        match self.0.ready(handle) {
            1 => Ok(true),
            0 => Ok(false),
            _ => Err(self.0.error(handle)),
        }
    }

    fn latency(&self, handle: u32) -> u32 {
        self.0.latency(handle)
    }

    fn take_wake(&self, handle: u32) -> bool {
        self.0.take_wake(handle)
    }

    fn reset(&self, handle: u32) {
        self.0.reset(handle);
    }

    fn destroy(&self, handle: u32) {
        self.0.destroy(handle);
    }

    fn save_state(&self, handle: u32) -> Result<Option<Vec<u8>>, String> {
        match self.0.save_state(handle) {
            Ok(v) if v.is_null() || v.is_undefined() => Ok(None),
            Ok(v) => {
                let bytes = js_sys::Uint8Array::new(&v);
                // Checked before the copy into this memory.
                if bytes.length() as usize > super::worklet::MAX_STATE_BYTES {
                    return Err(format!(
                        "state larger than {} bytes",
                        super::worklet::MAX_STATE_BYTES
                    ));
                }
                Ok(Some(bytes.to_vec()))
            }
            Err(e) => Err(e
                .as_string()
                .or_else(|| {
                    e.dyn_ref::<js_sys::Error>()
                        .map(|e| String::from(e.message()))
                })
                .unwrap_or_else(|| "the plugin failed to save its state".into())),
        }
    }

    fn param_values(&self, handle: u32) -> Option<Vec<(u32, f64)>> {
        let v = self.0.param_values(handle);
        if v.is_null() || v.is_undefined() {
            return None;
        }
        let flat = js_sys::Float64Array::new(&v).to_vec();
        Some(
            flat.as_chunks::<2>()
                .0
                .iter()
                .map(|[id, v]| (*id as u32, *v))
                .collect(),
        )
    }

    fn take_notifications(&self, out: &mut Vec<(u32, GuestNote)>) {
        let v = self.0.take_notifications();
        if v.is_undefined() || v.is_null() {
            return;
        }
        let list = js_sys::Array::from(&v);
        for entry in list.iter() {
            let e = js_sys::Array::from(&entry);
            let num = |i: u32| e.get(i).as_f64().unwrap_or(0.0);
            let handle = num(0) as u32;
            let (a, b) = (num(2) as u32, num(3));
            let note = match num(1) as u32 {
                note::PARAM => GuestNote::ParamEdited { id: a, value: b },
                note::BEGIN => GuestNote::GestureBegin { id: a },
                note::END => GuestNote::GestureEnd { id: a },
                note::DIRTY => GuestNote::StateDirty,
                note::PARAMS => GuestNote::ParamsChanged,
                note::CRASHED => GuestNote::Crashed {
                    message: e.get(4).as_string().unwrap_or_default(),
                },
                _ => continue,
            };
            out.push((handle, note));
        }
    }
}
