//! Main-thread half of an in-process VST2 plugin ([`Vst2Plugin`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use ether_core::config::PrepareConfig;
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::devices::{
    DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::ParamId;

use crate::abi::*;
use crate::gui::{DEFAULT_SIZE, HostWindow, Size};
use crate::host::Edit;
use crate::module::{Effect, Library};
use crate::node::{NodeInit, NodeShared, Precision, Vst2Node};
use crate::scan::{Info, info};

/// State blob framing: magic, version, kind, current program, then the payload.
const STATE_MAGIC: &[u8; 8] = b"EthVST2\0";
const STATE_VERSION: u32 = 1;
const KIND_PARAMS: u8 = 0;
const KIND_CHUNK: u8 = 1;

/// A VST2 state as saved by [`Vst2Plugin::save_state`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum State {
    /// `effGetChunk` (bank) of a plugin with `effFlagsProgramChunks`.
    Chunk { program: i32, data: Vec<u8> },
    /// Every parameter value (plugins without chunks).
    Params { program: i32, values: Vec<f32> },
}

impl State {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&STATE_VERSION.to_le_bytes());
        match self {
            State::Chunk { program, data } => {
                out.push(KIND_CHUNK);
                out.extend_from_slice(&program.to_le_bytes());
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(data);
            }
            State::Params { program, values } => {
                out.push(KIND_PARAMS);
                out.extend_from_slice(&program.to_le_bytes());
                out.extend_from_slice(&(values.len() as u32).to_le_bytes());
                for v in values {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        out
    }

    pub fn decode(blob: &[u8]) -> Result<Self, PluginError> {
        let bad = |what: &str| PluginError::State(format!("invalid VST2 state blob: {what}"));
        let rest = blob.strip_prefix(STATE_MAGIC).ok_or_else(|| bad("magic"))?;
        let u32_at = |r: &[u8], at: usize| -> Option<u32> {
            r.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let version = u32_at(rest, 0).ok_or_else(|| bad("truncated"))?;
        if version != STATE_VERSION {
            return Err(bad(&format!("unknown version {version}")));
        }
        let kind = *rest.get(4).ok_or_else(|| bad("truncated"))?;
        let program = u32_at(rest, 5).ok_or_else(|| bad("truncated"))? as i32;
        let len = u32_at(rest, 9).ok_or_else(|| bad("truncated"))? as usize;
        let payload = &rest[13..];
        match kind {
            KIND_CHUNK => {
                let data = payload.get(..len).ok_or_else(|| bad("truncated"))?;
                Ok(State::Chunk {
                    program,
                    data: data.to_vec(),
                })
            }
            KIND_PARAMS => {
                let bytes = payload
                    .get(..len.checked_mul(4).ok_or_else(|| bad("length"))?)
                    .ok_or_else(|| bad("truncated"))?;
                let values = bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                Ok(State::Params { program, values })
            }
            k => Err(bad(&format!("unknown kind {k}"))),
        }
    }
}

struct Editor {
    window: HostWindow,
}

/// An in-process VST2 plugin instance (main-thread half). Not `Send`: create and use it on
/// the plugin main thread (where its dispatcher calls and editor live).
pub struct Vst2Plugin {
    path: PathBuf,
    plugin_id: String,
    info: Info,
    params: Vec<ParamInfo>,
    /// Values right after opening (VST2 has no declared defaults).
    defaults: Vec<f32>,
    /// `(inputs, outputs, latency)` last seen (for `audioMasterIOChanged`).
    io: (i32, i32, u32),
    link: Option<Arc<NodeShared>>,
    editor: Option<Editor>,
    _not_send: std::marker::PhantomData<*const ()>,
    /// Dropped last (closes the plugin when the node is gone too).
    effect: Arc<Effect>,
}

impl std::fmt::Debug for Vst2Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vst2Plugin")
            .field("path", &self.path)
            .field("plugin_id", &self.plugin_id)
            .field("active", &self.link.is_some())
            .finish()
    }
}

impl Vst2Plugin {
    /// Load `path` and open the plugin `plugin_id` (main thread). For a shell library the id
    /// selects the sub-plugin (`audioMasterCurrentId`).
    pub fn load(path: &Path, plugin_id: &str) -> Result<Self, PluginError> {
        let uid = crate::parse_plugin_id(plugin_id)
            .ok_or_else(|| PluginError::NotFound(format!("invalid VST2 plugin id {plugin_id}")))?;
        let lib = Arc::new(Library::load(path)?);
        let effect = Effect::create(lib, uid)?;
        if effect.category() == kPlugCategShell {
            return Err(PluginError::NotFound(format!(
                "{plugin_id}: the shell {} did not create this sub-plugin",
                path.display()
            )));
        }
        if effect.unique_id() != uid {
            tracing::warn!(
                "VST2 {}: asked for {plugin_id}, the plugin reports {}",
                path.display(),
                crate::plugin_id(effect.unique_id())
            );
        }
        let info = info(&effect, path, None);
        let n = effect.num_params();
        let defaults = (0..n).map(|i| effect.get_parameter(i)).collect();
        let io = (effect.num_inputs(), effect.num_outputs(), effect.initial_delay());
        let mut plugin = Self {
            path: path.to_path_buf(),
            plugin_id: plugin_id.to_owned(),
            info,
            params: Vec::new(),
            defaults,
            io,
            link: None,
            editor: None,
            _not_send: std::marker::PhantomData,
            effect: Arc::new(effect),
        };
        plugin.params = plugin.list_params();
        Ok(plugin)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn is_active(&self) -> bool {
        self.link.is_some()
    }

    /// The plugin's latency (`initialDelay`) as last read.
    pub fn latency(&self) -> u32 {
        self.io.2
    }

    /// `effGetTailSize` in samples (`None` = unknown/default; `Some(u32::MAX)`-ish values
    /// mean "infinite"). Not part of the format-agnostic contract yet.
    pub fn tail_samples(&self) -> Option<u32> {
        match self.effect.dispatch(effGetTailSize, 0, 0, std::ptr::null_mut(), 0.0) {
            0 => None,
            1 => Some(0), // "no tail"
            n if n > 0 => Some(u32::try_from(n).unwrap_or(u32::MAX)),
            _ => None,
        }
    }

    /// The plugin's own text for a param: `(display, label)`, e.g. `("-6.0", "dB")`
    /// (`effGetParamDisplay` / `effGetParamLabel`). Main thread.
    pub fn param_text(&self, param: ParamId) -> Option<(String, String)> {
        let i = i32::try_from(param.0).ok()?;
        (i < self.effect.num_params()).then(|| {
            (
                self.effect.string(effGetParamDisplay, i),
                self.effect.string(effGetParamLabel, i),
            )
        })
    }

    /// `effVendorSpecific` (plugin-defined extensions). Main thread.
    pub fn vendor_specific(&mut self, index: i32, value: isize, opt: f32) -> isize {
        self.effect
            .dispatch(effVendorSpecific, index, value, std::ptr::null_mut(), opt)
    }

    /// The precision the node will process with.
    pub fn precision(&self) -> Option<Precision> {
        Precision::choose(&self.effect)
    }

    fn list_params(&self) -> Vec<ParamInfo> {
        (0..self.effect.num_params())
            .map(|i| {
                let name = match self.effect.string(effGetParamName, i) {
                    n if n.is_empty() => format!("Param {}", i + 1),
                    n => n,
                };
                ParamInfo {
                    id: ParamId(i as u32),
                    name,
                    group: None,
                    unit: ParamUnit::None,
                    min: 0.0,
                    max: 1.0,
                    default: f64::from(
                        self.defaults
                            .get(i as usize)
                            .copied()
                            .unwrap_or(0.0)
                            .clamp(0.0, 1.0),
                    ),
                    scale: ParamScale::Linear,
                    labels: None,
                    // `effCanBeAutomated` is unimplemented (0) in most plugins: every VST2
                    // param is automatable, as in other hosts.
                    automatable: true,
                    hidden: false,
                    step: None,
                    remote: None,
                }
            })
            .collect()
    }

    fn main_channels(&self) -> (u16, u16) {
        (
            self.effect.num_inputs().min(2) as u16,
            self.effect.num_outputs().min(2) as u16,
        )
    }

    fn close_editor_inner(&mut self) -> bool {
        let Some(editor) = self.editor.take() else {
            return false;
        };
        // The plugin's view goes first, then its parent window.
        self.effect
            .dispatch(effEditClose, 0, 0, std::ptr::null_mut(), 0.0);
        editor.window.close();
        true
    }

    fn editor_rect(&self) -> Option<Size> {
        let mut rect: *mut ERect = std::ptr::null_mut();
        self.effect.dispatch(
            effEditGetRect,
            0,
            0,
            (&mut rect as *mut *mut ERect).cast(),
            0.0,
        );
        // SAFETY: the plugin returns a pointer to its own rect (or leaves null).
        let r = unsafe { rect.as_ref() }?;
        let (w, h) = (
            i32::from(r.right) - i32::from(r.left),
            i32::from(r.bottom) - i32::from(r.top),
        );
        (w > 0 && h > 0).then_some(Size {
            width: w as u32,
            height: h as u32,
        })
    }

    fn poll_editor(&mut self, out: &mut Vec<PluginNotification>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if !editor.window.is_visible() {
            self.close_editor_inner();
            out.push(PluginNotification::EditorClosed);
            return;
        }
        self.effect
            .dispatch(effEditIdle, 0, 0, std::ptr::null_mut(), 0.0);
        if let Some((width, height)) = self.effect.host().take_resize() {
            editor.window.resize(Size { width, height });
        }
    }

    fn current_program(&self) -> i32 {
        self.effect
            .dispatch(effGetProgram, 0, 0, std::ptr::null_mut(), 0.0) as i32
    }
}

impl PluginController for Vst2Plugin {
    fn descriptor(&self) -> DeviceDescriptor {
        let (i, o) = self.main_channels();
        DeviceDescriptor {
            layout: None,
            device_type: DeviceTypeRef::Plugin {
                plugin_id: self.plugin_id.clone(),
            },
            name: self.info.name.clone(),
            category: self.info.category,
            params: self.params.clone(),
            audio_inputs: i,
            audio_outputs: o,
            midi_input: self.info.midi_input,
            // VST2 has no standard sidechain bus (extra inputs get silence).
            sidechain_inputs: 0,
        }
    }

    fn params(&mut self) -> Vec<ParamInfo> {
        self.params = self.list_params();
        self.params.clone()
    }

    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError> {
        if self.link.is_some() {
            return Err(PluginError::Activation("plugin is already active".into()));
        }
        let precision = Precision::choose(&self.effect).ok_or_else(|| {
            PluginError::Activation("plugin has no process function".into())
        })?;
        let e = &self.effect;
        let null = std::ptr::null_mut();
        let max_frames = config.max_block_size.max(1);
        e.host().set_setup(config.sample_rate, max_frames as u32);
        e.dispatch(effSetSampleRate, 0, 0, null, config.sample_rate);
        e.dispatch(effSetBlockSize, 0, max_frames as isize, null, 0.0);
        let p = if precision == Precision::Double {
            kVstProcessPrecision64
        } else {
            kVstProcessPrecision32
        };
        e.dispatch(effSetProcessPrecision, 0, p, null, 0.0);
        e.dispatch(effMainsChanged, 0, 1, null, 0.0);
        e.dispatch(effStartProcess, 0, 0, null, 0.0);
        // `initialDelay` is only reliable once resumed.
        self.io = (e.num_inputs(), e.num_outputs(), e.initial_delay());
        let shared = Arc::new(NodeShared::default());
        shared.latency.store(self.io.2, Ordering::Relaxed);
        self.link = Some(shared.clone());
        Ok(Box::new(Vst2Node::new(NodeInit {
            effect: self.effect.clone(),
            shared,
            descriptor: self.descriptor(),
            precision,
            config: *config,
        })))
    }

    fn deactivate(&mut self, node: Box<dyn PluginNode>) {
        drop(node);
        if self.link.take().is_some() {
            let null = std::ptr::null_mut();
            self.effect.dispatch(effStopProcess, 0, 0, null, 0.0);
            self.effect.dispatch(effMainsChanged, 0, 0, null, 0.0);
        }
    }

    fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
        let program = self.current_program();
        if self.effect.has_flag(effFlagsProgramChunks) {
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            // index 0 = the whole bank (1 = the current program only).
            let len = self.effect.dispatch(
                effGetChunk,
                0,
                0,
                (&mut data as *mut *mut std::ffi::c_void).cast(),
                0.0,
            );
            if len > 0 && !data.is_null() {
                // SAFETY: the plugin owns `len` bytes at `data` until its next call.
                let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) };
                return Ok(State::Chunk {
                    program,
                    data: bytes.to_vec(),
                }
                .encode());
            }
        }
        let values = (0..self.effect.num_params())
            .map(|i| self.effect.get_parameter(i))
            .collect();
        Ok(State::Params { program, values }.encode())
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if state.is_empty() {
            return Ok(());
        }
        let state = State::decode(state)?;
        let e = &self.effect;
        let null = std::ptr::null_mut();
        e.host().suppressed(|| match &state {
            State::Chunk { program, data } => {
                let mut data = data.clone();
                e.dispatch(
                    effSetChunk,
                    0,
                    data.len() as isize,
                    data.as_mut_ptr().cast(),
                    0.0,
                );
                if *program >= 0 && *program < e.num_programs() {
                    e.dispatch(effSetProgram, 0, *program as isize, null, 0.0);
                }
            }
            State::Params { program, values } => {
                if *program >= 0 && *program < e.num_programs() {
                    e.dispatch(effSetProgram, 0, *program as isize, null, 0.0);
                }
                for (i, v) in values.iter().enumerate() {
                    e.set_parameter(i as i32, *v);
                }
            }
        });
        Ok(())
    }

    fn param_value(&mut self, param: ParamId) -> Option<f64> {
        let i = i32::try_from(param.0).ok()?;
        (i < self.effect.num_params()).then(|| f64::from(self.effect.get_parameter(i)))
    }

    /// VST2 has a single object: `setParameter` works active or inactive.
    fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        let i = i32::try_from(param.0)
            .ok()
            .filter(|i| *i < self.effect.num_params())
            .ok_or_else(|| PluginError::State(format!("unknown param {}", param.0)))?;
        let e = &self.effect;
        e.host().suppressed(|| e.set_parameter(i, value as f32));
        Ok(())
    }

    fn has_editor(&self) -> bool {
        HostWindow::SUPPORTED && self.effect.has_flag(effFlagsHasEditor)
    }

    fn open_editor(&mut self) -> Result<(), PluginError> {
        if !self.has_editor() {
            return Err(PluginError::NoEditor);
        }
        if !HostWindow::on_main_thread() {
            return Err(PluginError::Load(
                "editor: plugin editors must be opened on the main thread".into(),
            ));
        }
        if let Some(e) = &self.editor {
            e.window.show();
            return Ok(());
        }
        let editor_err = |e: String| PluginError::Load(format!("editor: {e}"));
        let size = self.editor_rect().unwrap_or(DEFAULT_SIZE);
        // VST2 has no host-driven editor resize: the window follows the plugin only.
        let mut window = HostWindow::open(&self.info.name, size, false).map_err(editor_err)?;
        let Some(parent) = window.view_ptr() else {
            window.close();
            return Err(editor_err("no parent view".into()));
        };
        self.effect.host().take_resize();
        self.effect.dispatch(effEditOpen, 0, 0, parent, 0.0);
        // Many plugins only know their size once open.
        if let Some(actual) = self.editor_rect()
            && actual != size
        {
            window.resize(actual);
        }
        window.show();
        self.editor = Some(Editor { window });
        Ok(())
    }

    fn close_editor(&mut self) {
        self.close_editor_inner();
    }

    fn poll(&mut self, out: &mut Vec<PluginNotification>) {
        let host = self.effect.host().clone();
        for edit in host.take_edits() {
            out.push(match edit {
                Edit::Begin(i) => PluginNotification::GestureBegin { param: ParamId(i) },
                Edit::End(i) => PluginNotification::GestureEnd { param: ParamId(i) },
                Edit::Perform(i, v) => PluginNotification::ParamEdited {
                    param: ParamId(i),
                    value: f64::from(v.clamp(0.0, 1.0)),
                },
            });
        }
        if host.take_io_changed() {
            let io = (
                self.effect.num_inputs(),
                self.effect.num_outputs(),
                self.effect.initial_delay(),
            );
            let old = std::mem::replace(&mut self.io, io);
            if io.2 != old.2 {
                if let Some(link) = &self.link {
                    link.latency.store(io.2, Ordering::Relaxed);
                }
                out.push(PluginNotification::LatencyChanged { samples: io.2 });
            }
            if (io.0, io.1) != (old.0, old.1) {
                self.params = self.list_params();
                out.push(PluginNotification::ParamsChanged);
                if self.link.is_some() {
                    // New channel counts need new buffers.
                    out.push(PluginNotification::RestartRequested);
                }
            }
        }
        if host.take_update_display() {
            // Program change or internal edit: params may have new names, values changed.
            let params = self.list_params();
            if params.iter().map(|p| &p.name).ne(self.params.iter().map(|p| &p.name)) {
                self.params = params;
                out.push(PluginNotification::ParamsChanged);
            }
            out.push(PluginNotification::StateDirty);
        }
        self.poll_editor(out);
    }
}

impl Drop for Vst2Plugin {
    fn drop(&mut self) {
        self.close_editor_inner();
        if self.link.take().is_some() {
            // The node should have been deactivated; don't leave the plugin resumed.
            let null = std::ptr::null_mut();
            self.effect.dispatch(effStopProcess, 0, 0, null, 0.0);
            self.effect.dispatch(effMainsChanged, 0, 0, null, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_framing_round_trip() {
        for s in [
            State::Chunk {
                program: 3,
                data: b"chunk".to_vec(),
            },
            State::Chunk {
                program: 0,
                data: vec![],
            },
            State::Params {
                program: -1,
                values: vec![0.25, 1.0],
            },
        ] {
            let blob = s.encode();
            assert_eq!(State::decode(&blob).unwrap(), s);
            assert!(State::decode(&blob[..blob.len() - 1]).is_err());
        }
        assert!(State::decode(b"nope").is_err());
        let mut v2 = State::Params {
            program: 0,
            values: vec![],
        }
        .encode();
        v2[8] = 2;
        assert!(State::decode(&v2).is_err());
        let mut kind = State::Params {
            program: 0,
            values: vec![],
        }
        .encode();
        kind[12] = 9;
        assert!(State::decode(&kind).is_err());
    }
}
