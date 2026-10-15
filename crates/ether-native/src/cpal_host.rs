//! cpal hosts by name, with **one ASIO host for the whole process**.
//!
//! ASIO loads one driver per process, but each cpal ASIO `Host` only tracks the driver it
//! loaded itself. Listing devices (the settings dialog) or opening the input through a
//! second host loaded the other drivers one by one, which unloaded the driver the output
//! stream was playing through; the next start then found no driver ("audio device not
//! found: default output"). Through the shared host, loading another driver while one is
//! open fails instead (that device is skipped) and the open one is handed back.
//!
//! ASIO drivers are COM objects, and the ASIO SDK initializes COM only on the thread that
//! first used it. Every audio start runs on a new thread, so after the first one no driver
//! could be created ("no ASIO driver could be opened" until Ethereal restarted): COM is
//! initialized on each thread that gets the ASIO host.

use std::ops::Deref;

use cpal::traits::{DeviceTrait, HostTrait};

use crate::audio::AudioError;

/// A cpal host: owned, or the process-wide ASIO one.
pub(crate) enum HostRef {
    Owned(cpal::Host),
    #[cfg(windows)]
    Shared(&'static cpal::Host),
}

impl Deref for HostRef {
    type Target = cpal::Host;

    fn deref(&self) -> &cpal::Host {
        match self {
            Self::Owned(h) => h,
            #[cfg(windows)]
            Self::Shared(h) => h,
        }
    }
}

/// The host called `name` (case-insensitive), or the platform default.
pub(crate) fn host(name: Option<&str>) -> Result<HostRef, AudioError> {
    let Some(name) = name else {
        return Ok(HostRef::Owned(cpal::default_host()));
    };
    let id = cpal::available_hosts()
        .into_iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))
        .ok_or_else(|| AudioError::DeviceNotFound(format!("audio host {name}")))?;
    #[cfg(windows)]
    if id == cpal::HostId::Asio {
        com::initialize_thread();
        return asio_host().map(HostRef::Shared);
    }
    cpal::host_from_id(id)
        .map(HostRef::Owned)
        .map_err(|e| AudioError::Backend(e.to_string()))
}

#[cfg(windows)]
fn asio_host() -> Result<&'static cpal::Host, AudioError> {
    static HOST: std::sync::OnceLock<cpal::Host> = std::sync::OnceLock::new();
    if let Some(h) = HOST.get() {
        return Ok(h);
    }
    let h =
        cpal::host_from_id(cpal::HostId::Asio).map_err(|e| AudioError::Backend(e.to_string()))?;
    Ok(HOST.get_or_init(|| h))
}

/// Whether `host` is ASIO (one driver at a time; input and output on the same driver).
pub(crate) fn is_asio(host: &cpal::Host) -> bool {
    #[cfg(windows)]
    {
        host.id() == cpal::HostId::Asio
    }
    #[cfg(not(windows))]
    {
        let _ = host;
        false
    }
}

/// Whether the system, not the app, chooses the device buffer size on `host`: WASAPI in
/// shared mode (what cpal opens) calls back once per Windows engine period (~10 ms), whatever
/// buffer is asked for; a request only grows the queue (latency) behind it. Streams there
/// open with the default buffer and the settings say so.
pub(crate) fn system_buffer_size(host: &cpal::Host) -> bool {
    #[cfg(windows)]
    {
        host.id() == cpal::HostId::Wasapi
    }
    #[cfg(not(windows))]
    {
        let _ = host;
        false
    }
}

/// The installed ASIO drivers (from the registry, without loading them).
pub(crate) fn asio_driver_names() -> Vec<String> {
    #[cfg(windows)]
    {
        com::initialize_thread();
        asio_sys::Asio::new().driver_names()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// The output device called `name`, or the default one. ASIO has no default device: the
/// first driver that opens and has outputs.
pub(crate) fn output_device(
    host: &cpal::Host,
    name: Option<&str>,
) -> Result<cpal::Device, AudioError> {
    let asio = is_asio(host);
    let found = match name {
        Some(name) => host
            .output_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .find(|d| d.name().is_ok_and(|n| n == name)),
        None if asio => host
            .output_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .find(|d| d.default_output_config().is_ok()),
        None => host.default_output_device(),
    };
    found.ok_or_else(|| not_found(asio, name, "output"))
}

/// "Not found", with what to check on ASIO (where it mostly means the driver didn't open).
pub(crate) fn not_found(asio: bool, name: Option<&str>, what: &str) -> AudioError {
    AudioError::DeviceNotFound(match (asio, name) {
        (true, Some(name)) => format!(
            "{name}: the ASIO driver could not be opened (is the interface connected, and not in use by another application?)"
        ),
        (true, None) => {
            "no ASIO driver could be opened (is the interface connected, and not in use by another application?)"
                .into()
        }
        (false, Some(name)) => name.to_string(),
        (false, None) => format!("default {what}"),
    })
}

/// COM for the calling thread, once (uninitialized when the thread exits).
#[cfg(windows)]
mod com {
    use std::cell::Cell;

    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
    };

    /// Whether this thread's COM was initialized here (`Some(true)`: to uninitialize).
    struct Guard(Cell<Option<bool>>);

    impl Drop for Guard {
        fn drop(&mut self) {
            if self.0.get() == Some(true) {
                // SAFETY: balances the successful `CoInitializeEx` of this thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    thread_local! {
        static GUARD: Guard = const { Guard(Cell::new(None)) };
    }

    pub(super) fn initialize_thread() {
        GUARD.with(|g| {
            if g.0.get().is_none() {
                // Apartment-threaded, like the ASIO SDK's own `CoInitialize(0)`. Already
                // initialized differently (RPC_E_CHANGED_MODE) is fine: COM is usable.
                // SAFETY: plain call; balanced in `Guard::drop` when it succeeded.
                let hr =
                    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
                g.0.set(Some(hr >= 0));
            }
        });
    }
}
