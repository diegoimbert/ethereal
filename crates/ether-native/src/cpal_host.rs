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

// cpal 0.17 deprecates `Device::name()` for `id()` / `description()`. Saved devices are
// matched by id ([`find_device`]); the name is still what the settings show and the
// fallback for settings saved before ids were.
#![allow(deprecated)]

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

/// The output device saved as `id` (else called `name`), or the default one. ASIO has no
/// default device: the first driver that opens and has outputs.
pub(crate) fn output_device(
    host: &cpal::Host,
    name: Option<&str>,
    id: Option<&str>,
) -> Result<cpal::Device, AudioError> {
    let asio = is_asio(host);
    let found = match name {
        Some(name) => find_device(
            host.output_devices()
                .map_err(|e| AudioError::Backend(e.to_string()))?,
            name,
            id,
        ),
        None if asio => host
            .output_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .find(|d| d.default_output_config().is_ok()),
        None => host.default_output_device(),
    };
    found.ok_or_else(|| not_found(asio, name, "output"))
}

/// The device of `devices` whose [`device_id`] is `id`, else the one called `name` (settings
/// saved before ids were, or an id the host no longer knows).
pub(crate) fn find_device(
    devices: impl Iterator<Item = cpal::Device>,
    name: &str,
    id: Option<&str>,
) -> Option<cpal::Device> {
    let devices: Vec<cpal::Device> = devices.collect();
    let listed: Vec<Listed> = devices.iter().map(Listed::of).collect();
    let at = pick(&listed, name, id)?;
    devices.into_iter().nth(at)
}

/// A listed device's name and id.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Listed {
    pub name: Option<String>,
    pub id: Option<String>,
}

impl Listed {
    pub(crate) fn of(device: &cpal::Device) -> Self {
        Self {
            name: device.name().ok(),
            id: device_id(device),
        }
    }
}

/// [`find_device`] over names and ids: the index of the device with `id`, else of the one
/// called `name`.
pub(crate) fn pick(listed: &[Listed], name: &str, id: Option<&str>) -> Option<usize> {
    id.and_then(|id| listed.iter().position(|d| d.id.as_deref() == Some(id)))
        .or_else(|| listed.iter().position(|d| d.name.as_deref() == Some(name)))
}

/// The id a device is saved by: cpal's `DeviceId` (`host:id`: the ALSA PCM, the CoreAudio
/// UID, the WASAPI endpoint id), stable across restarts and renames.
pub(crate) fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

/// Whether `host` is ALSA (it lists every PCM of `aplay -L`, see [`alsa_listed`]).
pub(crate) fn is_alsa(host: &cpal::Host) -> bool {
    host.id().name().eq_ignore_ascii_case("alsa")
}

/// The device buffer (frames) a stream asks for when the settings name none; `None`: the
/// host's own default. cpal 0.17's ALSA default is the device's period (twice over), which
/// can be thousands of frames on a bare `hw:` device; the other hosts' defaults are what
/// their users expect (CoreAudio: the device's current size, shared by every app; WASAPI:
/// the system's period; ASIO: the driver panel's size; JACK: the server's, the only size
/// it accepts).
pub(crate) fn default_buffer_frames(host: &cpal::Host) -> Option<u32> {
    is_alsa(host).then_some(ALSA_DEFAULT_BUFFER)
}

/// [`default_buffer_frames`] on ALSA: 512 frames, ~11 ms at 48 kHz (the usual DAW default).
pub(crate) const ALSA_DEFAULT_BUFFER: u32 = 512;

/// Which ALSA PCMs (`pcms`, cpal's device names) the settings list unless all are asked
/// for: the sound server (`default`, `pipewire`, `pulse`) and each hardware device once
/// (`hw:CARD=<card>,DEV=<n>`; the card-number duplicates cpal adds, `hw:CARD=0,DEV=0`, only
/// when no named one is listed). Dropped: `plughw:`, the channel layouts (`front:`,
/// `surround51:`, ...), `sysdefault:`, `dmix:`/`dsnoop:`, the `iec958:`/`hdmi:` aliases and
/// the rate/mix plugins (`lavrate`, `upmix`, `null`, ...). When none would be kept, all are.
pub(crate) fn alsa_listed(pcms: &[String]) -> Vec<bool> {
    fn hw_card(pcm: &str) -> Option<&str> {
        pcm.strip_prefix("hw:CARD=")?.split(',').next()
    }
    let numeric = |card: &str| card.bytes().all(|b| b.is_ascii_digit());
    let named = pcms.iter().filter_map(|p| hw_card(p)).any(|c| !numeric(c));
    let keep: Vec<bool> = pcms
        .iter()
        .map(|p| match p.as_str() {
            "default" | "pipewire" | "pulse" => true,
            p => hw_card(p).is_some_and(|c| !(named && numeric(c))),
        })
        .collect();
    if keep.contains(&true) {
        keep
    } else {
        vec![true; pcms.len()]
    }
}

/// The devices the settings list (all of them with `all`, or off ALSA): see [`shown`].
/// Returns them with how many were left out.
pub(crate) fn shown_devices(
    host: &cpal::Host,
    devices: impl Iterator<Item = cpal::Device>,
    chosen: Option<(&str, Option<&str>)>,
    all: bool,
) -> (Vec<cpal::Device>, u32) {
    let devices: Vec<cpal::Device> = devices.collect();
    if all || !is_alsa(host) {
        return (devices, 0);
    }
    let listed: Vec<Listed> = devices.iter().map(Listed::of).collect();
    let keep = shown(&listed, chosen);
    let hidden = keep.iter().filter(|k| !**k).count() as u32;
    let kept = devices
        .into_iter()
        .zip(keep)
        .filter_map(|(d, k)| k.then_some(d))
        .collect();
    (kept, hidden)
}

/// Which ALSA devices are listed: [`alsa_listed`], plus the `chosen` one (name, id), so a
/// device picked from the full list (or saved before the filter) stays in view.
pub(crate) fn shown(listed: &[Listed], chosen: Option<(&str, Option<&str>)>) -> Vec<bool> {
    let pcms: Vec<String> = listed
        .iter()
        .map(|d| d.name.clone().unwrap_or_default())
        .collect();
    let mut keep = alsa_listed(&pcms);
    if let Some(at) = chosen.and_then(|(name, id)| pick(listed, name, id)) {
        keep[at] = true;
    }
    keep
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What cpal 0.17 lists on a laptop with PipeWire, an HDMI output and a USB interface
    /// (`aplay -L`, then the card-number `hw:`/`plughw:` duplicates cpal adds).
    const LAPTOP: &[&str] = &[
        "null",
        "pipewire",
        "default",
        "lavrate",
        "samplerate",
        "speexrate",
        "pulse",
        "upmix",
        "vdownmix",
        "hw:CARD=PCH,DEV=0",
        "hw:CARD=PCH,DEV=3",
        "plughw:CARD=PCH,DEV=0",
        "plughw:CARD=PCH,DEV=3",
        "sysdefault:CARD=PCH",
        "front:CARD=PCH,DEV=0",
        "surround21:CARD=PCH,DEV=0",
        "surround51:CARD=PCH,DEV=0",
        "surround71:CARD=PCH,DEV=0",
        "hdmi:CARD=PCH,DEV=0",
        "dmix:CARD=PCH,DEV=0",
        "dsnoop:CARD=PCH,DEV=0",
        "hw:CARD=USB,DEV=0",
        "plughw:CARD=USB,DEV=0",
        "sysdefault:CARD=USB",
        "iec958:CARD=USB,DEV=0",
        "hw:CARD=0,DEV=0",
        "plughw:CARD=0,DEV=0",
        "hw:CARD=1,DEV=0",
        "plughw:CARD=1,DEV=0",
    ];

    fn kept(names: &[&str]) -> Vec<String> {
        let all: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        let keep = alsa_listed(&all);
        all.into_iter()
            .zip(keep)
            .filter_map(|(n, k)| k.then_some(n))
            .collect()
    }

    #[test]
    fn alsa_lists_the_sound_server_and_each_hardware_device_once() {
        assert_eq!(
            kept(LAPTOP),
            [
                "pipewire",
                "default",
                "pulse",
                "hw:CARD=PCH,DEV=0",
                "hw:CARD=PCH,DEV=3",
                "hw:CARD=USB,DEV=0",
            ]
        );
    }

    #[test]
    fn alsa_without_named_cards_keeps_the_numbered_ones() {
        assert_eq!(
            kept(&["default", "hw:CARD=0,DEV=0", "plughw:CARD=0,DEV=0"]),
            ["default", "hw:CARD=0,DEV=0"]
        );
        // Nothing useful: everything rather than an empty list.
        assert_eq!(
            kept(&["dmix", "front:CARD=X,DEV=0"]),
            ["dmix", "front:CARD=X,DEV=0"]
        );
    }

    fn listed(names: &[&str]) -> Vec<Listed> {
        names
            .iter()
            .map(|n| Listed {
                name: Some(n.to_string()),
                id: Some(format!("alsa:{n}")),
            })
            .collect()
    }

    #[test]
    fn the_chosen_device_stays_listed() {
        let devices = listed(LAPTOP);
        let front = LAPTOP.iter().position(|n| n.starts_with("front:")).unwrap();
        assert!(!shown(&devices, None)[front]);
        // Picked from the full list, or saved under cpal 0.16: kept in view (by id or name).
        assert!(shown(&devices, Some(("front:CARD=PCH,DEV=0", None)))[front]);
        let by_id = shown(
            &devices,
            Some(("renamed", Some("alsa:front:CARD=PCH,DEV=0"))),
        );
        assert!(by_id[front]);
        assert_eq!(by_id.iter().filter(|k| **k).count(), 7);
    }

    #[test]
    fn devices_match_by_id_then_by_name() {
        let mut devices = listed(&["Speakers", "Headphones", "Speakers"]);
        devices[2].id = Some("coreaudio:usb-speakers".into());
        // The id wins over a name several devices share.
        assert_eq!(
            pick(&devices, "Speakers", Some("coreaudio:usb-speakers")),
            Some(2)
        );
        // Renamed since it was saved: still found by id.
        assert_eq!(pick(&devices, "Old name", Some("alsa:Headphones")), Some(1));
        // Saved by name only (or an id the host no longer lists): the name.
        assert_eq!(pick(&devices, "Headphones", None), Some(1));
        assert_eq!(pick(&devices, "Headphones", Some("wasapi:gone")), Some(1));
        assert_eq!(pick(&devices, "Gone", Some("wasapi:gone")), None);
    }
}
