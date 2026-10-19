//! Loading a `.vst3` module: locate the binary inside the bundle, call the platform entry
//! point (`bundleEntry` / `ModuleEntry` / `InitDll`), get the `IPluginFactory`, and call the
//! matching exit function when the module is dropped.
//!
//! Bundle layouts (VST3 SDK "Plug-in Format Structure"):
//! - macOS: `X.vst3/Contents/MacOS/<CFBundleExecutable>` (a CFBundle).
//! - Linux: `X.vst3/Contents/<arch>-linux/X.so`.
//! - Windows: `X.vst3/Contents/<arch>-win/X.vst3`.
//! - Legacy (Windows/Linux): `X.vst3` is the library itself.
//!
//! Instances share their module ([`Module::shared`]): a shell like Waves' exposes hundreds
//! of classes from one factory, and enumerating them, entering the module and setting the
//! host context are paid once per loaded module, not once per instance.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use ether_core::plugin::PluginError;
use vst3::Steinberg::{IPluginFactory, IPluginFactory3, IPluginFactory3Trait};
use vst3::{ComPtr, ComWrapper};

use crate::host::HostApplication;
use crate::scan::ClassInfo;

type GetFactory = unsafe extern "system" fn() -> *mut IPluginFactory;
type ExitFn = unsafe extern "system" fn() -> bool;

/// A loaded VST3 module. Drop order: factory, exit call, then the library.
pub(crate) struct Module {
    factory: Option<ComPtr<IPluginFactory>>,
    /// The factory's classes, enumerated on first use (plugin main thread).
    classes: OnceLock<Vec<ClassInfo>>,
    /// The context given to `IPluginFactory3`, kept for the module's lifetime in case the
    /// factory holds it without a reference of its own.
    host: Option<ComWrapper<HostApplication>>,
    exit: Option<ExitFn>,
    // Kept alive for the plugin's lifetime (the entry point got a reference to it).
    #[cfg(target_os = "macos")]
    _bundle: objc2_core_foundation::CFRetained<objc2_core_foundation::CFBundle>,
    _lib: libloading::Library,
    bundle: PathBuf,
}

// SAFETY: the library handle and the CFBundle are thread-safe; the factory and the host context
// are only called on the plugin main thread (`classes` is filled there too; `OnceLock` makes reading it from
// anywhere sound). Other threads (a node on the audio/GC thread) only keep the module alive
// through `Arc<Module>`, so the last one to drop runs the exit function.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module")
            .field("bundle", &self.bundle)
            .finish()
    }
}

thread_local! {
    /// Modules loaded for instances, by bundle path. Only used on the plugin main thread
    /// (where instances are created). Weak: a module still unloads (exit function, then the
    /// library) when its last instance and node are gone; the next instance loads it again.
    static SHARED: RefCell<HashMap<PathBuf, Weak<Module>>> = RefCell::new(HashMap::new());
}

impl Module {
    /// The module of `bundle` shared by every live instance (plugin main thread): loaded,
    /// entered and given the host context only when no instance holds it already.
    pub fn shared(bundle: &Path) -> Result<Arc<Self>, PluginError> {
        if let Some(module) = SHARED.with(|m| m.borrow().get(bundle).and_then(Weak::upgrade)) {
            return Ok(module);
        }
        // No borrow while plugin code runs: an entry point may pump a nested run loop that
        // instantiates another plugin (worst case both load; the last one is remembered).
        let mut module = Self::load(bundle)?;
        let host = ComWrapper::new(HostApplication);
        module.set_host_context(&host);
        module.host = Some(host);
        let module = Arc::new(module);
        SHARED.with(|m| {
            let mut m = m.borrow_mut();
            m.retain(|_, w| w.strong_count() > 0);
            m.insert(bundle.to_path_buf(), Arc::downgrade(&module));
        });
        Ok(module)
    }

    /// Load the module of `bundle` (runs plugin code: the entry point and factory getter).
    pub fn load(bundle: &Path) -> Result<Self, PluginError> {
        if !bundle.exists() {
            return Err(PluginError::NotFound(bundle.display().to_string()));
        }
        let binary = binary_path(bundle)?;

        #[cfg(target_os = "macos")]
        let cf_bundle = {
            use objc2_core_foundation::{CFBundle, CFURL};
            let url = CFURL::from_directory_path(bundle)
                .ok_or_else(|| PluginError::Load("bundle path is not a valid URL".into()))?;
            CFBundle::new(None, Some(&url))
                .ok_or_else(|| PluginError::Load("not a macOS bundle (CFBundleCreate)".into()))?
        };

        // SAFETY: loading a plugin library runs foreign code. Inherent to plugin hosting:
        // scanning happens out-of-process and instantiation is the user's explicit choice.
        let lib = unsafe { libloading::Library::new(&binary) }
            .map_err(|e| PluginError::Load(format!("{}: {e}", binary.display())))?;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let (lib, dl_handle) = {
            let raw = libloading::os::unix::Library::from(lib).into_raw();
            // SAFETY: `raw` came from `into_raw` just above.
            let lib = unsafe { libloading::os::unix::Library::from_raw(raw) };
            (libloading::Library::from(lib), raw)
        };

        let sym = |names: &[&[u8]]| -> Option<*mut c_void> {
            names.iter().find_map(|name| {
                // SAFETY: we only read the symbol address; the type is given at the call site.
                unsafe { lib.get::<*mut c_void>(*name) }
                    .ok()
                    .map(|s| *s)
                    .filter(|p| !p.is_null())
            })
        };

        #[cfg(target_os = "macos")]
        let (entry, exit) = (
            sym(&[b"bundleEntry\0", b"BundleEntry\0"]),
            sym(&[b"bundleExit\0", b"BundleExit\0"]),
        );
        #[cfg(target_os = "windows")]
        let (entry, exit) = (sym(&[b"InitDll\0"]), sym(&[b"ExitDll\0"]));
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let (entry, exit) = (sym(&[b"ModuleEntry\0"]), sym(&[b"ModuleExit\0"]));

        let get = sym(&[b"GetPluginFactory\0"])
            .ok_or_else(|| PluginError::Load("missing GetPluginFactory".into()))?;

        // SAFETY (all transmutes below): the SDK defines these symbols with exactly these
        // signatures (`bool PLUGIN_API f(...)`, `IPluginFactory* PLUGIN_API GetPluginFactory()`).
        let exit: Option<ExitFn> = exit.map(|p| unsafe { std::mem::transmute(p) });

        #[cfg(target_os = "macos")]
        let entered = match entry {
            Some(p) => {
                let f: unsafe extern "system" fn(*mut c_void) -> bool =
                    unsafe { std::mem::transmute(p) };
                let bundle_ref = objc2_core_foundation::CFRetained::as_ptr(&cf_bundle);
                unsafe { f(bundle_ref.as_ptr().cast()) }
            }
            None => return Err(PluginError::Load("missing bundleEntry".into())),
        };
        #[cfg(target_os = "windows")]
        let entered = match entry {
            // `InitDll` is optional on Windows.
            Some(p) => {
                let f: unsafe extern "system" fn() -> bool = unsafe { std::mem::transmute(p) };
                unsafe { f() }
            }
            None => true,
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let entered = match entry {
            Some(p) => {
                let f: unsafe extern "system" fn(*mut c_void) -> bool =
                    unsafe { std::mem::transmute(p) };
                // The SDK passes the `dlopen` handle.
                unsafe { f(dl_handle) }
            }
            None => return Err(PluginError::Load("missing ModuleEntry".into())),
        };
        if !entered {
            return Err(PluginError::Load("module entry function failed".into()));
        }

        let mut module = Self {
            factory: None,
            classes: OnceLock::new(),
            host: None,
            exit,
            #[cfg(target_os = "macos")]
            _bundle: cf_bundle,
            _lib: lib,
            bundle: bundle.to_path_buf(),
        };
        let get: GetFactory = unsafe { std::mem::transmute(get) };
        // SAFETY: GetPluginFactory returns an owned (already add-ref'd) factory or null.
        let factory = unsafe { ComPtr::from_raw(get()) }
            .ok_or_else(|| PluginError::Load("GetPluginFactory returned null".into()))?;
        module.factory = Some(factory);
        Ok(module)
    }

    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().expect("factory is set until drop")
    }

    /// Every class of the factory, enumerated once (plugin main thread).
    pub fn classes(&self) -> &[ClassInfo] {
        self.classes.get_or_init(|| crate::scan::classes(self))
    }

    /// Hand the host context to an `IPluginFactory3` (optional per the SDK).
    fn set_host_context(&self, host: &ComWrapper<HostApplication>) {
        if let Some(f3) = self.factory().cast::<IPluginFactory3>()
            && let Some(ctx) = host.as_com_ref::<vst3::Steinberg::FUnknown>()
        {
            // SAFETY: valid factory and host context; the factory add-refs what it keeps.
            unsafe { f3.setHostContext(ctx.as_ptr()) };
        }
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        self.factory = None;
        if let Some(exit) = self.exit {
            // SAFETY: balanced with the successful entry call in `load`.
            unsafe { exit() };
        }
    }
}

/// The module binary inside `bundle` for this OS/arch (see the module docs).
pub(crate) fn binary_path(bundle: &Path) -> Result<PathBuf, PluginError> {
    if bundle.is_file() {
        // Legacy single-file module (Windows/Linux).
        return Ok(bundle.to_path_buf());
    }
    let contents = bundle.join("Contents");
    let stem = bundle
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        use objc2_core_foundation::{CFBundle, CFURL};
        let from_plist = CFURL::from_directory_path(bundle)
            .and_then(|url| CFBundle::new(None, Some(&url)))
            .and_then(|b| b.executable_url())
            .and_then(|u| u.to_file_path())
            .filter(|p| p.is_file());
        if let Some(p) = from_plist {
            return Ok(p);
        }
        let p = contents.join("MacOS").join(&stem);
        if p.is_file() {
            return Ok(p);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let ext = if cfg!(windows) { "vst3" } else { "so" };
        let file = format!("{stem}.{ext}");
        for arch in arch_dirs() {
            let p = contents.join(arch).join(&file);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(PluginError::Unsupported(format!(
        "{} has no module binary for this platform ({})",
        bundle.display(),
        std::env::consts::ARCH
    )))
}

/// `Contents/<dir>` names holding this platform's binary, best match first.
#[cfg(not(target_os = "macos"))]
fn arch_dirs() -> &'static [&'static str] {
    match (std::env::consts::ARCH, cfg!(windows)) {
        ("x86_64", true) => &["x86_64-win"],
        ("aarch64", true) => &["arm64-win", "arm64x-win"],
        ("x86", true) => &["x86-win"],
        ("x86_64", false) => &["x86_64-linux"],
        ("aarch64", false) => &["aarch64-linux"],
        ("x86", false) => &["i386-linux", "i686-linux"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::AUDIO_MODULE_CLASS;
    use crate::testing;

    #[test]
    fn instances_share_the_module_until_the_last_one_drops() {
        let dir = testing::temp_dir("vst3-shared-module");
        let bundle = testing::make_bundle(&dir, "EtherVst3Shared");
        let a = Module::shared(&bundle).expect("load");
        let b = Module::shared(&bundle).expect("shared");
        assert!(Arc::ptr_eq(&a, &b));
        // Enumerated once, then served from the cache.
        let classes = a.classes();
        assert!(classes.iter().any(|c| c.category == AUDIO_MODULE_CLASS));
        assert!(std::ptr::eq(classes, b.classes()));
        let weak = Arc::downgrade(&a);
        drop((a, b));
        // The last instance gone: unloaded (exit function ran), loaded again on demand.
        assert!(weak.upgrade().is_none());
        let c = Module::shared(&bundle).expect("reload");
        assert!(!c.classes().is_empty());
        drop(c);
        let _ = std::fs::remove_dir_all(dir);
    }
}
