//! wasm-bindgen surface: JS glue types (SharedArrayBuffer ring memory, sync OPFS file
//! system) and the two exported classes.

use ether_controller::store::StoreError;
use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::message::{
    CommandError, ErrorCode, Event, NotificationLevel, Reply, ReplyResult,
};
use ether_core::protocol::{ClientMessage, ServerMessage};
use js_sys::{Atomics, Int32Array, SharedArrayBuffer, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::bridge::{self, Shared, WebBridge};
use crate::ring::{HEADER_BYTES, RingMemory};
use crate::store::{Fs, FsEntry, WebLibrary, WebStore};
use crate::worklet::EngineHost;

/// Ring memory over a `SharedArrayBuffer` (layout in [`crate::ring`]).
pub struct SabMemory {
    ctrl: Int32Array,
    data: Uint8Array,
    capacity: usize,
}

impl SabMemory {
    pub fn new(sab: &SharedArrayBuffer) -> Result<Self, JsError> {
        let capacity = (sab.byte_length() as usize).saturating_sub(HEADER_BYTES);
        if !capacity.is_power_of_two() || capacity < 16 {
            return Err(JsError::new(
                "ring SharedArrayBuffer must be 16 + 2^n bytes (n >= 4)",
            ));
        }
        Ok(Self {
            ctrl: Int32Array::new_with_byte_offset_and_length(sab, 0, 4),
            data: Uint8Array::new_with_byte_offset_and_length(
                sab,
                HEADER_BYTES as u32,
                capacity as u32,
            ),
            capacity,
        })
    }

    fn load(&self, i: u32) -> u32 {
        Atomics::load(&self.ctrl, i).unwrap_or(0) as u32
    }

    fn store(&self, i: u32, v: u32) {
        let _ = Atomics::store(&self.ctrl, i, v as i32);
    }
}

impl RingMemory for SabMemory {
    fn capacity(&self) -> usize {
        self.capacity
    }
    fn load_head(&self) -> u32 {
        self.load(0)
    }
    fn store_head(&self, v: u32) {
        self.store(0, v)
    }
    fn load_tail(&self) -> u32 {
        self.load(1)
    }
    fn store_tail(&self, v: u32) {
        self.store(1, v)
    }
    fn write(&self, pos: usize, bytes: &[u8]) {
        self.data
            .subarray(pos as u32, (pos + bytes.len()) as u32)
            .copy_from(bytes);
    }
    fn read(&self, pos: usize, out: &mut [u8]) {
        self.data
            .subarray(pos as u32, (pos + out.len()) as u32)
            .copy_to(out);
    }
}

#[wasm_bindgen]
extern "C" {
    /// Synchronous file system implemented in JS over OPFS (`apps/web/src/engine/syncFs.ts`).
    /// Methods throw `{ code: "NotFound" | "Io" | ..., message }` on failure.
    #[derive(Clone)]
    pub type JsFsHost;

    #[wasm_bindgen(method, catch)]
    fn read(this: &JsFsHost, path: &str) -> Result<Uint8Array, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn write(this: &JsFsHost, path: &str, bytes: &[u8]) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn rename(this: &JsFsHost, from: &str, to: &str) -> Result<(), JsValue>;
    /// JSON array of `{ name, is_dir, size, modified_ms }`.
    #[wasm_bindgen(method, catch)]
    fn list(this: &JsFsHost, dir: &str) -> Result<String, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn mkdir(this: &JsFsHost, path: &str) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn remove(this: &JsFsHost, path: &str) -> Result<(), JsValue>;
    /// JSON entry or `null`.
    #[wasm_bindgen(method, catch)]
    fn stat(this: &JsFsHost, path: &str) -> Result<String, JsValue>;
    /// `length` bytes of a file from `offset` (fewer at the end).
    #[wasm_bindgen(method, catch, js_name = readRange)]
    fn read_range(
        this: &JsFsHost,
        path: &str,
        offset: f64,
        length: u32,
    ) -> Result<Uint8Array, JsValue>;
}

/// [`Fs`] over the JS sync file system.
#[derive(Clone)]
pub struct JsFs(pub JsFsHost);

fn js_err(e: JsValue) -> StoreError {
    let get = |k: &str| {
        js_sys::Reflect::get(&e, &JsValue::from_str(k))
            .ok()
            .and_then(|v| v.as_string())
    };
    let message = get("message").unwrap_or_else(|| format!("{e:?}"));
    match get("code").or_else(|| get("name")).as_deref() {
        Some("NotFound" | "NotFoundError") => StoreError::NotFound(message),
        Some("AlreadyExists") => StoreError::AlreadyExists(message),
        Some("InvalidPath" | "TypeError") => StoreError::InvalidPath(message),
        Some("Unsupported") => StoreError::Unsupported(message),
        _ => StoreError::Io(message),
    }
}

#[derive(serde::Deserialize)]
struct JsEntry {
    name: String,
    is_dir: bool,
    size: f64,
    modified_ms: f64,
}

impl From<JsEntry> for FsEntry {
    fn from(e: JsEntry) -> Self {
        FsEntry {
            name: e.name,
            is_dir: e.is_dir,
            size: e.size as u64,
            modified_ms: e.modified_ms,
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, StoreError> {
    serde_json::from_str(json).map_err(|e| StoreError::Io(format!("fs host: {e}")))
}

impl Fs for JsFs {
    fn read(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.0.read(path).map(|a| a.to_vec()).map_err(js_err)
    }
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.0.write(path, bytes).map_err(js_err)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreError> {
        self.0.rename(from, to).map_err(js_err)
    }
    fn list(&mut self, dir: &str) -> Result<Vec<FsEntry>, StoreError> {
        let entries: Vec<JsEntry> = parse(&self.0.list(dir).map_err(js_err)?)?;
        Ok(entries.into_iter().map(Into::into).collect())
    }
    fn mkdir(&mut self, path: &str) -> Result<(), StoreError> {
        self.0.mkdir(path).map_err(js_err)
    }
    fn remove(&mut self, path: &str) -> Result<(), StoreError> {
        self.0.remove(path).map_err(js_err)
    }
    fn stat(&mut self, path: &str) -> Result<Option<FsEntry>, StoreError> {
        let e: Option<JsEntry> = parse(&self.0.stat(path).map_err(js_err)?)?;
        Ok(e.map(Into::into))
    }
}

/// Ranged reads of OPFS media ([`crate::media_stream`]).
impl crate::media_stream::RangeFs for JsFs {
    fn size(&mut self, path: &str) -> Result<u64, String> {
        match self.stat(path) {
            Ok(Some(e)) => Ok(e.size),
            Ok(None) => Err(format!("{path}: not found")),
            Err(e) => Err(e.to_string()),
        }
    }
    fn read_range(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        self.0
            .read_range(path, offset as f64, len as u32)
            .map(|a| a.to_vec())
            .map_err(|e| js_err(e).to_string())
    }
}

/// HTTP requests of the sample pack, run by the controller Worker's JS
/// (`WasmController::take_http_requests` → `fetch` → `http_progress` / `http_done`).
#[derive(Default)]
pub struct HttpQueue {
    requests: Vec<(u64, String, u64)>,
    events: Vec<ether_controller::HttpEvent>,
}

/// Clock + entropy for the controller, and the sample pack's downloads.
pub struct WebHost {
    state: u64,
    http: std::rc::Rc<std::cell::RefCell<HttpQueue>>,
}

impl WebHost {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed | 1,
            http: Default::default(),
        }
    }
}

impl HostServices for WebHost {
    fn now_ms(&self) -> u64 {
        js_sys::Date::now() as u64
    }

    fn random_seed(&mut self) -> u64 {
        // splitmix64 over the seed JS drew from crypto.getRandomValues.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn http_get(&mut self, id: u64, url: &str, max_bytes: u64) -> Result<(), String> {
        self.http
            .borrow_mut()
            .requests
            .push((id, url.to_string(), max_bytes));
        Ok(())
    }

    fn poll_http(&mut self, out: &mut Vec<ether_controller::HttpEvent>) {
        out.append(&mut self.http.borrow_mut().events);
    }
}

/// The web build's controller.
pub type WebController =
    EtherController<WebBridge<SabMemory>, WebHost, WebStore<JsFs>, WebLibrary<JsFs>>;

/// Set up every web controller (its extensions, `EtherController::add_extension`;
/// docs/EXTENSIONS.md). Call once, before the UI creates the controller.
#[cfg(feature = "app-controller")]
pub fn set_controller_setup(setup: fn(&mut WebController)) {
    let _ = CONTROLLER_SETUP.set(setup);
}

#[cfg(feature = "app-controller")]
static CONTROLLER_SETUP: std::sync::OnceLock<fn(&mut WebController)> = std::sync::OnceLock::new();

/// Builds the web controller instead of the app's own (an extension's, with its state:
/// `EtherController::with_state`; docs/EXTENSIONS.md). Call once, before the UI creates it.
/// Without the `app-controller` feature, the module has no controller of its own: this is
/// required.
pub fn set_controller_factory(factory: WebControllerFactory) {
    let _ = CONTROLLER_FACTORY.set(factory);
}

/// What [`set_controller_factory`] takes.
pub type WebControllerFactory = fn(
    WebBridge<SabMemory>,
    WebHost,
    WebStore<JsFs>,
    WebLibrary<JsFs>,
    ControllerConfig,
) -> Box<dyn Controller>;

static CONTROLLER_FACTORY: std::sync::OnceLock<WebControllerFactory> = std::sync::OnceLock::new();

/// Controller side (Web Worker).
#[wasm_bindgen]
pub struct WasmController {
    controller: Box<dyn Controller>,
    shared: Shared<SabMemory>,
    reported_errors: usize,
    /// Reported as a notification with the first batch of messages.
    startup_warning: Option<String>,
    http: std::rc::Rc<std::cell::RefCell<HttpQueue>>,
}

#[wasm_bindgen]
impl WasmController {
    /// The real `EtherController`. `sample_rate`: the AudioContext rate the Worklet renders
    /// at (media is resampled to it by the controller). `control`/`reports`: the two ring
    /// buffers shared with the Worklet. `fs`: the sync OPFS file system (the demo samples
    /// are written into its sample library on first start).
    #[wasm_bindgen(constructor)]
    pub fn new(
        seed: u64,
        sample_rate: u32,
        control: &SharedArrayBuffer,
        reports: &SharedArrayBuffer,
        fs: JsFsHost,
    ) -> Result<WasmController, JsError> {
        console_error_panic_hook::set_once();
        let shared = bridge::shared(SabMemory::new(control)?, SabMemory::new(reports)?);
        let mut bridge = WebBridge::new(shared.clone());
        bridge.set_sample_rate(sample_rate);
        bridge.set_webclap_waiter(webclap_waiter());
        let mut fs = JsFs(fs);
        bridge.set_stream_opener(crate::media_stream::range_opener(fs.clone()));
        let startup_warning = crate::store::ensure_demo_samples(&mut fs)
            .err()
            .map(|e| format!("Demo samples could not be written: {e}"));
        let host = WebHost::new(seed);
        let http = host.http.clone();
        let config = ControllerConfig {
            engine_sample_rate: sample_rate,
            ..ControllerConfig::default()
        };
        let controller: Box<dyn Controller> = match CONTROLLER_FACTORY.get() {
            Some(factory) => factory(
                bridge,
                host,
                WebStore::new(fs.clone()),
                WebLibrary::new(fs),
                config,
            ),
            #[cfg(not(feature = "app-controller"))]
            None => {
                return Err(JsError::new(
                    "no controller: set_controller_factory was not called (no app-controller feature)",
                ));
            }
            #[cfg(feature = "app-controller")]
            None => {
                let mut ctl: WebController = EtherController::with_config(
                    bridge,
                    host,
                    WebStore::new(fs.clone()),
                    WebLibrary::new(fs),
                    config,
                );
                if let Some(setup) = CONTROLLER_SETUP.get() {
                    setup(&mut ctl);
                }
                Box::new(ctl)
            }
        };
        Ok(WasmController {
            controller,
            shared,
            reported_errors: 0,
            startup_warning,
            http,
        })
    }

    /// The HTTP GETs the controller asked for since the last call, as a JSON array of
    /// `{id, url, max}`; the Worker runs each with `fetch` and answers with
    /// [`Self::http_progress`] and [`Self::http_done`].
    pub fn take_http_requests(&mut self) -> String {
        let requests = std::mem::take(&mut self.http.borrow_mut().requests);
        let list: Vec<serde_json::Value> = requests
            .into_iter()
            .map(|(id, url, max)| serde_json::json!({ "id": id, "url": url, "max": max }))
            .collect();
        serde_json::Value::Array(list).to_string()
    }

    /// Body bytes received so far for request `id` (`total` < 0: unknown).
    pub fn http_progress(&mut self, id: f64, received: f64, total: f64) {
        self.http
            .borrow_mut()
            .events
            .push(ether_controller::HttpEvent::Progress {
                id: id as u64,
                received: received as u64,
                total: (total >= 0.0).then_some(total as u64),
            });
    }

    /// Request `id` finished: an HTTP `status` and `body`, or (`error` set) a network
    /// failure.
    pub fn http_done(&mut self, id: f64, status: u16, body: &[u8], error: Option<String>) {
        let result = match error {
            Some(e) => Err(e),
            None => Ok((status, body.to_vec())),
        };
        self.http
            .borrow_mut()
            .events
            .push(ether_controller::HttpEvent::Done {
                id: id as u64,
                result,
            });
    }

    /// Handle one JSON `ClientMessage`; returns a JSON array of `ServerMessage`s (patches
    /// first, then exactly one reply).
    pub fn handle(&mut self, message_json: &str) -> String {
        let mut out: Vec<ServerMessage> = Vec::new();
        match serde_json::from_str::<ClientMessage>(message_json) {
            Ok(msg) => self.controller.handle(msg, &mut out),
            Err(e) => {
                let id = serde_json::from_str::<serde_json::Value>(message_json)
                    .ok()
                    .and_then(|v| v.get("id")?.as_u64())
                    .unwrap_or(0) as u32;
                out.push(ServerMessage::Reply(Reply {
                    id,
                    result: ReplyResult::Err {
                        error: CommandError {
                            code: ErrorCode::InvalidArgument,
                            message: format!("malformed message: {e}"),
                            key: None,
                        },
                    },
                }));
            }
        }
        self.finish(out)
    }

    /// Periodic work (~60 Hz); returns a JSON array of `ServerMessage`s.
    pub fn tick(&mut self, now_ms: f64) -> String {
        let mut out: Vec<ServerMessage> = Vec::new();
        self.controller.tick(now_ms as u64, &mut out);
        self.finish(out)
    }

    /// The installed WebCLAP plugins: the page's JSON list of scanned modules
    /// (`crate::webclap::catalog::ScannedModule`). Replaces the previous list.
    pub fn set_webclap_catalog(&mut self, json: &str) -> Result<(), JsError> {
        let catalog = crate::webclap::catalog::Catalog::parse(json)
            .map_err(|e| JsError::new(&format!("bad webclap catalog: {e}")))?;
        self.shared.borrow_mut().webclap.catalog = catalog;
        Ok(())
    }

    /// This Worker's WebCLAP host (`WebclapHost`, offline renders).
    pub fn set_webclap_host(&mut self, host: crate::webclap::js::JsWebclapHost) {
        crate::webclap::guest::set_guest(Some(std::rc::Rc::new(crate::webclap::js::JsGuest(host))));
    }

    /// Control bytes not yet accepted by the Worklet (backpressure diagnostics).
    pub fn pending_control_bytes(&self) -> usize {
        self.shared.borrow().control.pending_bytes()
    }

    /// Blocks rendered by the Worklet as of the last report (0 until audio runs).
    pub fn engine_blocks(&self) -> f64 {
        self.shared.borrow().blocks as f64
    }

    fn finish(&mut self, mut out: Vec<ServerMessage>) -> String {
        let mut shared = self.shared.borrow_mut();
        shared.control.flush();
        if let Some(message) = self.startup_warning.take() {
            out.push(ServerMessage::Event(Event::Notification {
                level: NotificationLevel::Warning,
                message,
                key: None,
            }));
        }
        // Engine-side errors become warnings (at most a few per tick; the rest are counted).
        for e in shared.errors.drain(..) {
            self.reported_errors += 1;
            if self.reported_errors <= 100 {
                let m = ether_controller::text::msg("notice.audio.engine_error")
                    .arg("error", e.to_string());
                out.push(ServerMessage::Event(Event::Notification {
                    level: NotificationLevel::Warning,
                    message: ether_controller::text::english(&m),
                    key: Some(m),
                }));
            }
        }
        serde_json::to_string(&out).expect("server messages serialize")
    }
}

/// Engine side (AudioWorkletProcessor).
#[wasm_bindgen]
pub struct WasmEngine {
    host: EngineHost<SabMemory>,
}

#[wasm_bindgen]
impl WasmEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(
        sample_rate: u32,
        control: &SharedArrayBuffer,
        reports: &SharedArrayBuffer,
    ) -> Result<WasmEngine, JsError> {
        console_error_panic_hook::set_once();
        Ok(WasmEngine {
            host: EngineHost::new(
                sample_rate,
                SabMemory::new(control)?,
                SabMemory::new(reports)?,
            ),
        })
    }

    /// Render one block (`frames <= 128`) into the internal planar output buffers.
    pub fn process(&mut self, frames: usize) {
        self.host.render(frames);
    }

    /// Address (in wasm memory) of output channel `channel` (0 or 1), 128 floats.
    pub fn output_ptr(&self, channel: usize) -> *const f32 {
        self.host.output_ptr(channel)
    }

    pub fn blocks(&self) -> f64 {
        self.host.blocks() as f64
    }

    /// The Worklet's WebCLAP host (`WebclapHost` in `apps/web/src/engine/webclap/host.ts`).
    pub fn set_webclap_host(&mut self, host: crate::webclap::js::JsWebclapHost) {
        crate::webclap::guest::set_guest(Some(std::rc::Rc::new(crate::webclap::js::JsGuest(host))));
    }
}

/// How the controller Worker sleeps while it waits for a WebCLAP reply: `Atomics.wait` on a
/// buffer of its own (nothing ever notifies it, so it is a plain timed sleep).
fn webclap_waiter() -> Box<dyn FnMut(u32)> {
    let cell = js_sys::Int32Array::new(&js_sys::SharedArrayBuffer::new(4));
    Box::new(move |ms| {
        let _ = js_sys::Atomics::wait_with_timeout(&cell, 0, 0, f64::from(ms));
    })
}

/// `performance.now()` in milliseconds (Window, Worker, Node).
#[cfg(feature = "bench")]
fn performance_now() -> f64 {
    let perf = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("performance"))
        .expect("global performance");
    let now = js_sys::Reflect::get(&perf, &JsValue::from_str("now"))
        .expect("performance.now")
        .unchecked_into::<js_sys::Function>();
    now.call0(&perf)
        .ok()
        .and_then(|v| v.as_f64())
        .expect("performance.now() returns a number")
}

/// Graph snapshot costs of the large fixture, measured in wasm: JSON of
/// [`crate::perf::SnapshotCosts`], fastest of `runs`. Diagnostics only, not used by the app
/// (feature `bench`, not in release builds).
#[cfg(feature = "bench")]
#[wasm_bindgen]
pub fn bench_graph_snapshot(runs: u32) -> String {
    let costs = crate::perf::measure(runs as usize, performance_now);
    serde_json::to_string(&costs).expect("costs serialize")
}

/// Worklet quanta while a playing reverb switches to the longest factory IR, in
/// wasm: JSON of [`crate::perf::IrSwapCosts`], fastest of `runs`. Diagnostics only
/// (feature `bench`, not in release builds).
#[cfg(feature = "bench")]
#[wasm_bindgen]
pub fn bench_ir_swap(runs: u32) -> String {
    let costs = crate::perf::measure_ir_swap(runs as usize, performance_now);
    serde_json::to_string(&costs).expect("costs serialize")
}

/// Worklet quanta while a playing reverb switches to a 6 s stereo user IR, built by
/// the Worklet as before and prepared by the Worker: JSON of
/// [`crate::perf::MediaIrSwapCosts`], fastest of `runs`. Diagnostics only (feature `bench`).
#[cfg(feature = "bench")]
#[wasm_bindgen]
pub fn bench_media_ir_swap(runs: u32) -> String {
    let costs = crate::perf::measure_media_ir_swap(6.0, runs as usize, performance_now);
    serde_json::to_string(&costs).expect("costs serialize")
}

/// Stretcher costs per Worklet quantum (Complex, Complex+), in wasm: JSON of
/// [`crate::perf::StretchCosts`], fastest of `runs`. Diagnostics only (feature `bench`).
#[cfg(all(feature = "bench", feature = "warp-complex"))]
#[wasm_bindgen]
pub fn bench_stretch(runs: u32) -> String {
    let costs = crate::perf::measure_stretch(10.0, runs as usize, performance_now);
    serde_json::to_string(&costs).expect("costs serialize")
}
