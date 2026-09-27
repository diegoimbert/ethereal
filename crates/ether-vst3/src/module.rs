//! Loading a `.vst3` module: locate the binary inside the bundle, call the platform entry
//! point (`bundleEntry` / `ModuleEntry` / `InitDll`), get the `IPluginFactory`, and call the
//! matching exit function when the module is dropped.
//!
//! Bundle layouts (VST3 SDK "Plug-in Format Structure"):
//! - macOS: `X.vst3/Contents/MacOS/<CFBundleExecutable>` (a CFBundle).
//! - Linux: `X.vst3/Contents/<arch>-linux/X.so`.
//! - Windows: `X.vst3/Contents/<arch>-win/X.vst3`.
//! - Legacy (Windows/Linux): `X.vst3` is the library itself.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use ether_core::plugin::PluginError;
use vst3::ComPtr;
use vst3::Steinberg::{IPluginFactory, IPluginFactory3, IPluginFactory3Trait};

use crate::host::HostApplication;

type GetFactory = unsafe extern "system" fn() -> *mut IPluginFactory;
type ExitFn = unsafe extern "system" fn() -> bool;

/// A loaded VST3 module. Drop order: factory, exit call, then the library.
pub(crate) struct Module {
    factory: Option<ComPtr<IPluginFactory>>,
    exit: Option<ExitFn>,
    // Kept alive for the plugin's lifetime (the entry point got a reference to it).
    #[cfg(target_os = "macos")]
    _bundle: objc2_core_foundation::CFRetained<objc2_core_foundation::CFBundle>,
    _lib: libloading::Library,
    bundle: PathBuf,
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module").field("bundle", &self.bundle).finish()
    }
}

impl Module {
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
                unsafe { lib.get::<*mut c_void>(name) }
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

    /// Hand the host context to an `IPluginFactory3` (optional per the SDK).
    pub fn set_host_context(&self, host: &vst3::ComWrapper<HostApplication>) {
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
    #[cfg(all(windows, target_arch = "x86_64"))]
    return &["x86_64-win"];
    #[cfg(all(windows, target_arch = "aarch64"))]
    return &["arm64-win", "arm64x-win", "arm64ec-win"];
    #[cfg(all(windows, target_arch = "x86"))]
    return &["x86-win"];
    #[cfg(all(not(windows), target_arch = "x86_64"))]
    return &["x86_64-linux"];
    #[cfg(all(not(windows), target_arch = "aarch64"))]
    return &["aarch64-linux"];
    #[cfg(all(not(windows), target_arch = "x86"))]
    return &["i386-linux", "i686-linux"];
    #[allow(unreachable_code)]
    &[]
}
