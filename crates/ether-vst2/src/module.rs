//! Loading a VST2 library and creating `AEffect` instances from it.
//!
//! Shapes on disk:
//! - Windows: `X.dll`; Linux: `X.so` (the library itself);
//! - macOS: a `X.vst` CFBundle, binary at `Contents/MacOS/<CFBundleExecutable>`.
//!
//! Entry points, first found wins: `VSTPluginMain`, `main_macho` (old macOS builds), `main`
//! (pre-2.4 builds). Each is `AEffect* (audioMasterCallback)`. A library without any of them
//! is not a VST2 plugin ([`LoadError::NotVst2`]; folders of helper libraries are common).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;

use ether_core::plugin::PluginError;

use crate::abi::*;
use crate::host::{HostShared, host_callback, with_current_id};

/// Why a library could not be loaded.
#[derive(Debug)]
pub(crate) enum LoadError {
    /// No VST2 entry point: some other shared library.
    NotVst2,
    Plugin(PluginError),
}

impl From<LoadError> for PluginError {
    fn from(e: LoadError) -> Self {
        match e {
            LoadError::NotVst2 => PluginError::Unsupported("not a VST2 plugin".into()),
            LoadError::Plugin(e) => e,
        }
    }
}

/// A loaded VST2 library. Unloaded when the last [`Effect`] made from it is gone.
pub(crate) struct Library {
    main: PluginMain,
    // Kept alive while plugins run (resources looked up through the bundle).
    #[cfg(target_os = "macos")]
    _bundle: objc2_core_foundation::CFRetained<objc2_core_foundation::CFBundle>,
    _lib: libloading::Library,
    path: PathBuf,
}

// SAFETY: the library handle and the CFBundle are thread-safe; `main` is only called on the
// plugin main thread.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library").field("path", &self.path).finish()
    }
}

impl Library {
    /// Load `path` (runs the library's static initializers).
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        if !path.exists() {
            return Err(LoadError::Plugin(PluginError::NotFound(
                path.display().to_string(),
            )));
        }
        let binary = binary_path(path).map_err(LoadError::Plugin)?;
        #[cfg(target_os = "macos")]
        let cf_bundle = {
            use objc2_core_foundation::{CFBundle, CFURL};
            let url = CFURL::from_directory_path(path).ok_or_else(|| {
                LoadError::Plugin(PluginError::Load("bundle path is not a valid URL".into()))
            })?;
            CFBundle::new(None, Some(&url)).ok_or_else(|| {
                LoadError::Plugin(PluginError::Load("not a macOS bundle (CFBundleCreate)".into()))
            })?
        };
        // SAFETY: loading a plugin library runs foreign code. Inherent to plugin hosting:
        // scanning happens out-of-process and instantiation is the user's explicit choice.
        let lib = unsafe { libloading::Library::new(&binary) }.map_err(|e| {
            LoadError::Plugin(PluginError::Load(format!("{}: {e}", binary.display())))
        })?;
        let names: &[&[u8]] = &[b"VSTPluginMain\0", b"main_macho\0", b"main\0"];
        let main = names
            .iter()
            .find_map(|name| {
                // SAFETY: only the address is read; the signature is the ABI's entry type.
                unsafe { lib.get::<*mut c_void>(name) }
                    .ok()
                    .map(|s| *s)
                    .filter(|p| !p.is_null())
            })
            .ok_or(LoadError::NotVst2)?;
        // SAFETY: every VST2 entry point has the `PluginMain` signature.
        let main: PluginMain = unsafe { std::mem::transmute::<*mut c_void, PluginMain>(main) };
        Ok(Self {
            main,
            #[cfg(target_os = "macos")]
            _bundle: cf_bundle,
            _lib: lib,
            path: path.to_path_buf(),
        })
    }

}

/// The library binary for `path` (the path itself, or the executable of a macOS bundle).
pub(crate) fn binary_path(path: &Path) -> Result<PathBuf, PluginError> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_core_foundation::{CFBundle, CFURL};
        let from_plist = CFURL::from_directory_path(path)
            .and_then(|url| CFBundle::new(None, Some(&url)))
            .and_then(|b| b.executable_url())
            .and_then(|u| u.to_file_path())
            .filter(|p| p.is_file());
        if let Some(p) = from_plist {
            return Ok(p);
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let p = path.join("Contents/MacOS").join(stem);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(PluginError::Unsupported(format!(
        "{} has no plugin binary for this platform",
        path.display()
    )))
}

/// One open plugin instance (`AEffect` after `effOpen`). Shared by the controller and its
/// node (`Arc`); `effClose` runs when the last one drops, then the library may unload.
pub(crate) struct Effect {
    ptr: NonNull<AEffect>,
    host: Arc<HostShared>,
    /// Dropped last.
    lib: Arc<Library>,
}

// SAFETY: VST2 splits its calls by thread like every host does: the controller calls the
// dispatcher on the plugin main thread, the node calls process/events/params on the audio
// thread. The host state is thread-safe.
unsafe impl Send for Effect {}
unsafe impl Sync for Effect {}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effect")
            .field("library", &self.lib.path)
            .field("unique_id", &self.unique_id())
            .finish()
    }
}

impl Effect {
    /// Create and open an instance (plugin main thread). `shell_id` is the sub-plugin of a
    /// shell library to create (`audioMasterCurrentId`); 0 = the library's own plugin.
    pub fn create(lib: Arc<Library>, shell_id: i32) -> Result<Self, PluginError> {
        let dir = lib
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let host = Arc::new(HostShared::new(&dir));
        with_current_id(shell_id, || {
            // SAFETY: the entry point with our callback; returns an owned AEffect or null.
            let raw = unsafe { (lib.main)(host_callback) };
            let ptr = NonNull::new(raw)
                .ok_or_else(|| PluginError::Load("the plugin entry point returned null".into()))?;
            // SAFETY: a non-null AEffect from the entry point.
            let e = unsafe { &mut *ptr.as_ptr() };
            if e.magic != EFFECT_MAGIC {
                return Err(PluginError::Load("bad AEffect magic (not a VST2 plugin)".into()));
            }
            if e.dispatcher.is_none() {
                return Err(PluginError::Load("AEffect has no dispatcher".into()));
            }
            e.resvd2 = Arc::as_ptr(&host) as VstIntPtr;
            let effect = Self { ptr, host, lib };
            // Shells may still ask for the current id while opening.
            effect.dispatch(effOpen, 0, 0, std::ptr::null_mut(), 0.0);
            Ok(effect)
        })
    }

    pub fn raw(&self) -> *mut AEffect {
        self.ptr.as_ptr()
    }

    fn aeffect(&self) -> &AEffect {
        // SAFETY: valid until `effClose` (in drop).
        unsafe { self.ptr.as_ref() }
    }

    pub fn host(&self) -> &Arc<HostShared> {
        &self.host
    }

    pub fn dispatch(
        &self,
        opcode: i32,
        index: i32,
        value: VstIntPtr,
        ptr: *mut c_void,
        opt: f32,
    ) -> VstIntPtr {
        match self.aeffect().dispatcher {
            // SAFETY: the plugin's dispatcher on its own instance.
            Some(d) => unsafe { d(self.raw(), opcode, index, value, ptr, opt) },
            None => 0,
        }
    }

    /// A string opcode (`effGetParamName`, `effGetEffectName`, ...) into a generous buffer.
    pub fn string(&self, opcode: i32, index: i32) -> String {
        let mut buf = [0u8; STRING_BUF];
        self.dispatch(opcode, index, 0, buf.as_mut_ptr().cast(), 0.0);
        // Never trust the plugin to have terminated it.
        buf[STRING_BUF - 1] = 0;
        read_cstr(&buf)
    }

    /// `effCanDo`: 1 = yes, -1 = no, 0 = don't know.
    pub fn can_do(&self, what: &std::ffi::CStr) -> VstIntPtr {
        self.dispatch(effCanDo, 0, 0, what.as_ptr() as *mut c_void, 0.0)
    }

    pub fn flags(&self) -> i32 {
        self.aeffect().flags
    }

    pub fn has_flag(&self, flag: i32) -> bool {
        self.flags() & flag != 0
    }

    pub fn num_inputs(&self) -> i32 {
        self.aeffect().num_inputs.max(0)
    }

    pub fn num_outputs(&self) -> i32 {
        self.aeffect().num_outputs.max(0)
    }

    pub fn num_params(&self) -> i32 {
        self.aeffect().num_params.max(0)
    }

    pub fn num_programs(&self) -> i32 {
        self.aeffect().num_programs.max(0)
    }

    pub fn initial_delay(&self) -> u32 {
        self.aeffect().initial_delay.max(0) as u32
    }

    pub fn unique_id(&self) -> i32 {
        self.aeffect().unique_id
    }

    pub fn version(&self) -> i32 {
        self.aeffect().version
    }

    pub fn get_parameter(&self, index: i32) -> f32 {
        match self.aeffect().get_parameter {
            // SAFETY: the plugin's accessor on its own instance.
            Some(f) if index >= 0 && index < self.num_params() => unsafe { f(self.raw(), index) },
            _ => 0.0,
        }
    }

    pub fn set_parameter(&self, index: i32, value: f32) {
        if let Some(f) = self.aeffect().set_parameter
            && index >= 0
            && index < self.num_params()
        {
            // SAFETY: the plugin's accessor on its own instance.
            unsafe { f(self.raw(), index, value.clamp(0.0, 1.0)) };
        }
    }

    pub fn process_replacing(&self) -> Option<ProcessProc> {
        self.aeffect().process_replacing
    }

    pub fn process_double_replacing(&self) -> Option<ProcessDoubleProc> {
        self.aeffect().process_double_replacing
    }

    pub fn process_accumulating(&self) -> Option<ProcessProc> {
        self.aeffect().process
    }

    /// `effGetPlugCategory`.
    pub fn category(&self) -> VstIntPtr {
        self.dispatch(effGetPlugCategory, 0, 0, std::ptr::null_mut(), 0.0)
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        // The plugin frees the AEffect in effClose: nothing touches it afterwards.
        self.dispatch(effClose, 0, 0, std::ptr::null_mut(), 0.0);
    }
}
