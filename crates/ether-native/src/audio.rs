//! Audio backends: a `cpal` output stream, or a device-less `null` / `offline` thread.
//!
//! All backends run the same [`RtRenderer`] (which owns the `Engine`). Stopping a backend
//! drops the renderer off the audio thread, which hands the engine back so it can move to
//! another device without being rebuilt (the sample rate must stay the same: changing it
//! means a new engine, see [`crate::host`]).
//!
//! - `cpal`: the stream is built and owned by a small control thread (cpal streams are not
//!   `Send` on every platform). The device buffer may be any size; the renderer splits it
//!   into engine blocks of at most `max_block_size` frames.
//! - `null`: a thread renders one block per block period at real-time pace (sleeping until
//!   the next deadline), with no device. Selected by `ETHER_AUDIO=null`; used by CI, tests
//!   and parallel dev instances so nobody contends for the sound card.
//! - `offline`: like `null` but as fast as possible (yielding between blocks).

// cpal 0.17 deprecates `Device::name()` for `id()` / `description()`: devices are saved by
// id ([`AudioSettings::output_device_id`]), the name is what the settings show.
#![allow(deprecated)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use ether_core::Engine;
use ether_core::protocol::engine::{AudioConfig, AudioDeviceInfo, AudioDeviceList};
use serde::{Deserialize, Serialize};

use crate::cpal_host;
use crate::rt::{AudioShared, RtRenderer, enable_flush_denormals};

/// Which audio backend to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioBackendKind {
    Cpal,
    /// Real-time paced, no device.
    Null,
    /// Render as fast as possible (tests, bounce).
    Offline,
}

impl AudioBackendKind {
    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s? {
            "cpal" => Some(Self::Cpal),
            "null" => Some(Self::Null),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cpal => "cpal",
            Self::Null => "null",
            Self::Offline => "offline",
        }
    }
}

/// Audio device settings (persisted by the host in `<data_dir>/config/audio.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub backend: AudioBackendKind,
    /// cpal host name (`CoreAudio`, `ALSA`, `JACK`, `WASAPI`, ...); `None` = default host.
    pub host: Option<String>,
    /// Output device name; `None` = default device.
    pub output_device: Option<String>,
    /// The output device's cpal id ([`crate::cpal_host::device_id`]), matched before the
    /// name (names change: cpal 0.17 renamed devices, the OS renames them). Recorded when
    /// the device opens ([`AudioSettings::with_device_ids`]); `None` for settings saved
    /// before ids were (matched by name, then given the id), and cleared when the device
    /// changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_device_id: Option<String>,
    /// Input device name (`None` = no input; `loopback[:<samples>]` = test loopback).
    pub input_device: Option<String>,
    /// The input device's cpal id, as [`AudioSettings::output_device_id`] (`None` for the
    /// default input, which follows the system).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_device_id: Option<String>,
    /// Requested sample rate; `None` = device default (48 kHz for null/offline).
    pub sample_rate: Option<u32>,
    /// Requested device buffer size in frames; `None` = device default (the engine block
    /// size for null/offline).
    pub buffer_size: Option<u32>,
    /// Engine max block size (`EngineConfig::max_block_size`; plugins are activated with
    /// it). Device buffers larger than this are rendered in several engine blocks.
    pub max_block_size: usize,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            backend: AudioBackendKind::Cpal,
            host: None,
            output_device: None,
            output_device_id: None,
            input_device: None,
            input_device_id: None,
            sample_rate: None,
            buffer_size: None,
            max_block_size: 1024,
        }
    }
}

impl AudioSettings {
    pub fn to_protocol(&self) -> AudioConfig {
        AudioConfig {
            backend: Some(self.backend.name().to_string()),
            host: self.host.clone(),
            output_device: self.output_device.clone(),
            input_device: self.input_device.clone(),
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
        }
    }

    /// Apply the `Some` fields of a protocol `AudioConfig`.
    pub fn merged(&self, c: &AudioConfig) -> Result<Self, AudioError> {
        let mut s = self.clone();
        if let Some(b) = &c.backend {
            s.backend = AudioBackendKind::parse(Some(b))
                .ok_or_else(|| AudioError::Config(format!("unknown backend {b}")))?;
        }
        if c.host.is_some() && c.host != s.host {
            s.host = c.host.clone();
            // Device names belong to a host (WASAPI "Speakers (…)" vs an ASIO driver):
            // unless given with it, the default output, and the default input if one was on.
            if c.output_device.is_none() {
                s.output_device = None;
            }
            if c.input_device.is_none()
                && s.input_device
                    .as_deref()
                    .is_some_and(|d| crate::recording::parse_loopback(d).is_none())
            {
                s.input_device = Some("default".into());
            }
        }
        if c.output_device.is_some() {
            s.output_device = c.output_device.clone();
        }
        if c.input_device.is_some() {
            s.input_device = c.input_device.clone().filter(|d| !d.is_empty());
        }
        if c.sample_rate.is_some() {
            s.sample_rate = c.sample_rate;
        }
        if c.buffer_size.is_some() {
            s.buffer_size = c.buffer_size;
        }
        // Another device: its id is recorded when it opens.
        if s.output_device != self.output_device {
            s.output_device_id = None;
        }
        if s.input_device != self.input_device {
            s.input_device_id = None;
        }
        Ok(s)
    }

    /// These settings with their devices' ids recorded, and their names refreshed from the
    /// devices found by id (what the settings list shows), if the cpal host lists them.
    /// Settings saved by name only (before ids were) get their ids here once: the
    /// migration. `None`: nothing to change. Not on ASIO (its id is the driver name, and
    /// listing loads the drivers).
    pub fn with_device_ids(&self) -> Option<Self> {
        if self.backend != AudioBackendKind::Cpal {
            return None;
        }
        let host = cpal_host::host(self.host.as_deref()).ok()?;
        if cpal_host::is_asio(&host) {
            return None;
        }
        let listed = |devices: Result<Vec<cpal::Device>, cpal::DevicesError>| {
            devices
                .map(|ds| ds.iter().map(cpal_host::Listed::of).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let mut s = self.clone();
        if s.output_device.is_some() {
            let outputs = listed(host.output_devices().map(Iterator::collect));
            identify(&mut s.output_device, &mut s.output_device_id, &outputs);
        }
        if s.input_device.as_deref().is_some_and(is_input_device_name) {
            let inputs = listed(host.input_devices().map(Iterator::collect));
            identify(&mut s.input_device, &mut s.input_device_id, &inputs);
        }
        (s != *self).then_some(s)
    }
}

/// An input setting naming a device (not the system default, not the test loopback).
fn is_input_device_name(name: &str) -> bool {
    name != "default" && crate::recording::parse_loopback(name).is_none()
}

/// Record the id of the device `name`/`id` designates among `listed`, and its current name.
/// A device not listed (unplugged) keeps what was saved.
fn identify(name: &mut Option<String>, id: &mut Option<String>, listed: &[cpal_host::Listed]) {
    let Some(n) = name.as_deref() else { return };
    let Some(at) = cpal_host::pick(listed, n, id.as_deref()) else {
        return;
    };
    let found = &listed[at];
    if found.id.is_some() {
        id.clone_from(&found.id);
    }
    if found.name.is_some() {
        name.clone_from(&found.name);
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AudioError {
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
    #[error("unsupported audio config: {0}")]
    Config(String),
    #[error("audio backend error: {0}")]
    Backend(String),
}

/// What a started stream actually runs at.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub backend: AudioBackendKind,
    pub device: Option<String>,
    pub sample_rate: u32,
    /// Device buffer size in frames (0 = backend default / unknown until the first callback).
    pub buffer_size: u32,
    pub channels: u16,
}

/// Resolved device configuration (cpal) or the thread's pacing (null/offline).
#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    pub info: StreamInfo,
}

/// Work out the sample rate/device a backend would run with, without starting it (the
/// engine is created with this sample rate).
pub fn resolve(settings: &AudioSettings) -> Result<ResolvedConfig, AudioError> {
    match settings.backend {
        AudioBackendKind::Null | AudioBackendKind::Offline => Ok(ResolvedConfig {
            info: StreamInfo {
                backend: settings.backend,
                device: None,
                sample_rate: settings.sample_rate.unwrap_or(48_000),
                buffer_size: settings
                    .buffer_size
                    .unwrap_or(settings.max_block_size as u32)
                    .clamp(16, settings.max_block_size.max(16) as u32),
                channels: 2,
            },
        }),
        AudioBackendKind::Cpal => {
            let (host, device) = cpal_device(settings)?;
            let system_buffer = cpal_host::system_buffer_size(&host);
            let settings = &with_host_buffer(settings, &host);
            let (config, _) = cpal_config(&device, settings, None, system_buffer)?;
            Ok(ResolvedConfig {
                info: StreamInfo {
                    backend: AudioBackendKind::Cpal,
                    device: device.name().ok(),
                    sample_rate: config.sample_rate,
                    buffer_size: match config.buffer_size {
                        cpal::BufferSize::Fixed(n) => n,
                        cpal::BufferSize::Default => 0,
                    },
                    channels: config.channels,
                },
            })
        }
    }
}

fn cpal_device(settings: &AudioSettings) -> Result<(cpal_host::HostRef, cpal::Device), AudioError> {
    let host = cpal_host::host(settings.host.as_deref())?;
    let device = cpal_host::output_device(
        &host,
        settings.output_device.as_deref(),
        settings.output_device_id.as_deref(),
    )?;
    Ok((host, device))
}

/// `settings` asking for the host's default buffer size when they ask for none
/// ([`cpal_host::default_buffer_frames`]; saved settings keep `None`).
fn with_host_buffer(settings: &AudioSettings, host: &cpal::Host) -> AudioSettings {
    AudioSettings {
        buffer_size: settings
            .buffer_size
            .or_else(|| cpal_host::default_buffer_frames(host)),
        ..settings.clone()
    }
}

/// Pick the stream config: the requested (or forced) sample rate if the device supports
/// it in its default sample format, else the device default; the requested buffer size
/// clamped into the device range (see [`pick_buffer_size`]; `system_buffer`: the driver
/// chooses, see [`cpal_host::system_buffer_size`]).
fn cpal_config(
    device: &cpal::Device,
    settings: &AudioSettings,
    force_rate: Option<u32>,
    system_buffer: bool,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), AudioError> {
    let default = device
        .default_output_config()
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    let format = default.sample_format();
    let wanted = force_rate.or(settings.sample_rate);
    let supported = match wanted {
        Some(rate) if rate != default.sample_rate() => device
            .supported_output_configs()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .filter(|c| c.sample_format() == format && c.channels() == default.channels())
            .find_map(|c| c.try_with_sample_rate(rate))
            .or_else(|| {
                device.supported_output_configs().ok()?.find_map(|c| {
                    c.try_with_sample_rate(rate)
                        .filter(|c| is_supported_format(c.sample_format()))
                })
            }),
        _ => Some(default.clone()),
    };
    let supported = match (supported, force_rate) {
        (Some(s), _) => s,
        (None, Some(rate)) => {
            return Err(AudioError::Config(format!(
                "device does not support {rate} Hz"
            )));
        }
        (None, None) => default,
    };
    let buffer_size =
        pick_buffer_size(settings.buffer_size, supported.buffer_size(), system_buffer);
    let format = supported.sample_format();
    if !is_supported_format(format) {
        return Err(AudioError::Config(format!("sample format {format:?}")));
    }
    let mut config = supported.config();
    config.buffer_size = buffer_size;
    Ok((config, format))
}

/// The device buffer a stream opens with: the requested size, clamped into the device's
/// range (a size it can't run used to fall back to the driver default silently, so a
/// change seemed to do nothing). `Default` when nothing was asked, or when the system sets
/// the buffer (`system_buffer`).
fn pick_buffer_size(
    requested: Option<u32>,
    supported: &cpal::SupportedBufferSize,
    system_buffer: bool,
) -> cpal::BufferSize {
    match (requested, supported) {
        (None, _) | (Some(0), _) => cpal::BufferSize::Default,
        _ if system_buffer => cpal::BufferSize::Default,
        (Some(n), cpal::SupportedBufferSize::Range { min, max }) if (*min).max(1) <= *max => {
            cpal::BufferSize::Fixed(n.clamp((*min).max(1), *max))
        }
        (Some(n), cpal::SupportedBufferSize::Unknown) => cpal::BufferSize::Fixed(n),
        _ => cpal::BufferSize::Default,
    }
}

/// The `[min, max]` buffer range of a device config, `None` when unknown or unconstrained
/// (WASAPI's software stack reports `0..=u32::MAX`).
fn buffer_size_range(supported: &cpal::SupportedBufferSize) -> Option<[u32; 2]> {
    match supported {
        cpal::SupportedBufferSize::Range { min, max } if *max != u32::MAX && min <= max => {
            Some([*min, *max])
        }
        _ => None,
    }
}

fn is_supported_format(f: cpal::SampleFormat) -> bool {
    matches!(
        f,
        cpal::SampleFormat::F32
            | cpal::SampleFormat::I16
            | cpal::SampleFormat::U16
            | cpal::SampleFormat::I32
            | cpal::SampleFormat::F64
    )
}

/// Enumerate cpal hosts and output devices (plus the null/offline backends). On ALSA only
/// the useful devices unless `all` ([`cpal_host::shown_devices`]).
pub fn list_devices(current: &AudioSettings, all: bool) -> AudioDeviceList {
    let mut outputs = Vec::new();
    let mut hidden = 0;
    let mut buffer_size_settable = true;
    if let Ok(host) = cpal_host::host(current.host.as_deref()) {
        let system_buffer = cpal_host::system_buffer_size(&host);
        buffer_size_settable = !system_buffer;
        // ASIO has no default device (and asking for one would load drivers).
        let default_name = if cpal_host::is_asio(&host) {
            None
        } else {
            host.default_output_device().and_then(|d| d.name().ok())
        };
        if let Ok(devices) = host.output_devices() {
            let chosen = current
                .output_device
                .as_deref()
                .map(|n| (n, current.output_device_id.as_deref()));
            let (devices, left_out) = cpal_host::shown_devices(&host, devices, chosen, all);
            hidden += left_out;
            for d in devices {
                let Ok(name) = d.name() else { continue };
                let default = d.default_output_config().ok();
                let mut rates: Vec<u32> = d
                    .supported_output_configs()
                    .map(|cs| {
                        cs.flat_map(|c| {
                            [44_100, 48_000, 88_200, 96_000, 176_400, 192_000]
                                .into_iter()
                                .filter(move |r| {
                                    c.min_sample_rate() <= *r && *r <= c.max_sample_rate()
                                })
                        })
                        .collect()
                    })
                    .unwrap_or_default();
                rates.sort_unstable();
                rates.dedup();
                outputs.push(AudioDeviceInfo {
                    is_default: default_name.as_deref() == Some(name.as_str()),
                    channels: default.as_ref().map_or(2, |c| c.channels()),
                    sample_rates: rates,
                    buffer_size_range: default
                        .as_ref()
                        .filter(|_| !system_buffer)
                        .and_then(|c| buffer_size_range(c.buffer_size())),
                    name,
                });
            }
        }
        if cpal_host::is_asio(&host) {
            push_unopened_asio_drivers(&mut outputs);
        }
    }
    let (inputs, inputs_hidden) = crate::recording::list_input_devices(current, all);
    AudioDeviceList {
        backends: vec!["cpal".into(), "null".into(), "offline".into()],
        hosts: cpal::available_hosts()
            .into_iter()
            .map(|h| h.name().to_string())
            .collect(),
        outputs,
        inputs,
        current: current.to_protocol(),
        buffer_size_settable: buffer_size_settable || current.backend != AudioBackendKind::Cpal,
        hidden_devices: hidden + inputs_hidden,
    }
}

/// ASIO drivers the listing couldn't open (one driver loads at a time: while a stream plays
/// through one, the others are skipped) are still offered, by name.
pub(crate) fn push_unopened_asio_drivers(list: &mut Vec<AudioDeviceInfo>) {
    for name in cpal_host::asio_driver_names() {
        if !list.iter().any(|d| d.name == name) {
            list.push(AudioDeviceInfo {
                is_default: false,
                channels: 2,
                sample_rates: Vec::new(),
                buffer_size_range: None,
                name,
            });
        }
    }
}

/// A running audio backend. [`AudioOutput::stop`] returns the engine.
pub struct AudioOutput {
    pub info: StreamInfo,
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
    engine_back: Receiver<Box<Engine>>,
}

impl std::fmt::Debug for AudioOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOutput")
            .field("info", &self.info)
            .finish()
    }
}

impl AudioOutput {
    /// Start `settings.backend` with `engine` (whose sample rate must be what [`resolve`]
    /// returned). On failure the engine is returned with the error.
    pub fn start(
        engine: Box<Engine>,
        settings: &AudioSettings,
        shared: Arc<AudioShared>,
    ) -> Result<Self, (AudioError, Box<Engine>)> {
        let (back_tx, back_rx) = unbounded();
        // The last callback's size is what the status reports: forget the previous
        // stream's, so a status read before the new stream's first callback shows the
        // size it was opened with, not the old one.
        shared.buffer_size.store(0, Ordering::Relaxed);
        // A new stream: the previous one's fate is handled (or moot).
        shared.device_lost.store(false, Ordering::SeqCst);
        shared.render_crashed.store(false, Ordering::SeqCst);
        crate::recording::configure_input(&shared, settings);
        let renderer = RtRenderer::new(engine, shared.clone(), back_tx);
        let result = match settings.backend {
            AudioBackendKind::Cpal => start_cpal(renderer, settings, shared.clone()),
            AudioBackendKind::Null | AudioBackendKind::Offline => {
                start_thread(renderer, settings, shared.clone())
            }
        };
        match result {
            Ok((info, stop, thread)) => {
                shared.running.store(true, Ordering::Relaxed);
                if settings.backend == AudioBackendKind::Cpal {
                    // The device decides the size actually run (a driver may round it, or
                    // pick its own with `Default`): wait for the first callback, so the
                    // status replied to a change reports it.
                    wait_first_callback(&shared, Duration::from_millis(250));
                }
                Ok(Self {
                    info,
                    stop,
                    thread: Some(thread),
                    engine_back: back_rx,
                })
            }
            Err(e) => {
                // The renderer was dropped by the failed start: take the engine back.
                let engine = back_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("engine returned by failed audio start");
                Err((e, engine))
            }
        }
    }

    /// Stop the stream and take the engine back (`None` only if the backend thread died).
    pub fn stop(mut self) -> Option<Box<Engine>> {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.engine_back.recv_timeout(Duration::from_secs(5)).ok()
    }
}

/// Wait (up to `timeout`) until the audio callback has run once (`buffer_size` set).
fn wait_first_callback(shared: &AudioShared, timeout: Duration) {
    let until = Instant::now() + timeout;
    while shared.buffer_size.load(Ordering::Relaxed) == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
}

type Started = (StreamInfo, Sender<()>, JoinHandle<()>);

fn start_thread(
    mut renderer: RtRenderer,
    settings: &AudioSettings,
    shared: Arc<AudioShared>,
) -> Result<Started, AudioError> {
    let info = resolve(settings)?.info;
    let block = info.buffer_size as usize;
    let paced = settings.backend == AudioBackendKind::Null;
    let period = Duration::from_secs_f64(block as f64 / info.sample_rate as f64);
    let (stop_tx, stop_rx) = bounded::<()>(1);
    let thread = std::thread::Builder::new()
        .name(format!("ether-audio-{}", settings.backend.name()))
        .spawn(move || {
            enable_flush_denormals();
            let mut buf = vec![0.0f32; block * 2];
            let mut deadline = Instant::now();
            loop {
                match stop_rx.try_recv() {
                    Ok(()) | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                    Err(crossbeam_channel::TryRecvError::Empty) => {}
                }
                renderer.render(&mut buf, 2);
                if paced {
                    deadline += period;
                    let now = Instant::now();
                    if deadline > now {
                        // Sleep until the next block is due (or a stop arrives).
                        if stop_rx.recv_timeout(deadline - now).is_ok() {
                            break;
                        }
                    } else if now - deadline > period * 4 {
                        // Fell far behind (suspended, overloaded): count a dropout and
                        // resync instead of rendering a burst.
                        shared.xruns.fetch_add(1, Ordering::Relaxed);
                        deadline = now;
                    }
                } else {
                    std::thread::yield_now();
                }
            }
            shared.running.store(false, Ordering::Relaxed);
            drop(renderer);
        })
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    Ok((info, stop_tx, thread))
}

fn start_cpal(
    renderer: RtRenderer,
    settings: &AudioSettings,
    shared: Arc<AudioShared>,
) -> Result<Started, AudioError> {
    let settings = settings.clone();
    let (stop_tx, stop_rx) = bounded::<()>(1);
    let (ready_tx, ready_rx) = bounded::<Result<StreamInfo, AudioError>>(1);
    let thread = std::thread::Builder::new()
        .name("ether-audio-cpal".into())
        .spawn(move || {
            let device = match cpal_device(&settings) {
                Ok(d) => d,
                Err(e) => {
                    drop(renderer);
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let asio = cpal_host::is_asio(&device.0);
            // Elsewhere the input opens first (it doesn't depend on the output).
            let mut _input = if asio {
                None
            } else {
                crate::recording::open_input(&settings, &shared, None)
            };
            let system_buffer = cpal_host::system_buffer_size(&device.0);
            let settings = with_host_buffer(&settings, &device.0);
            let (stream, buffer_frames) = match build_cpal(
                renderer,
                &settings,
                &device.1,
                system_buffer,
                callback_priority(asio),
                shared.clone(),
            ) {
                Ok((stream, info)) => {
                    // 0: the driver's default size (the input then asks for it too).
                    let frames = info.buffer_size;
                    let _ = ready_tx.send(Ok(info));
                    (stream, (frames > 0).then_some(frames))
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            if asio {
                // ASIO: the input is the output's driver (one driver at a time; cpal pairs
                // both streams on the same device and their buffers share one size). It
                // opens after the output, with the output's buffer size: opened first with
                // the driver's default size, the output then re-created the shared buffers
                // at its own size while the input callback kept a buffer sized for the old
                // one, and cpal, deriving the channel count from it, indexed past the
                // driver's channels (a panic in the ASIO callback: "index out of bounds").
                _input = crate::recording::open_input(
                    &settings,
                    &shared,
                    Some(crate::recording::AsioDuplex {
                        output: &device.1,
                        buffer_frames,
                    }),
                );
            }
            let _ = stop_rx.recv();
            shared.running.store(false, Ordering::Relaxed);
            // Dropping the stream stops the device and drops the callback (RtRenderer),
            // which sends the engine back.
            drop(stream);
        })
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    match ready_rx.recv() {
        Ok(Ok(info)) => Ok((info, stop_tx, thread)),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            let _ = thread.join();
            Err(AudioError::Backend("audio thread died".into()))
        }
    }
}

/// Whether the device callback's thread needs the workers' real-time promotion
/// (`crate::workers::promote_current_thread`): on Windows WASAPI (cpal only raises it to
/// `THREAD_PRIORITY_TIME_CRITICAL`, not into MMCSS) and on Linux (cpal leaves its ALSA thread
/// at normal priority). ASIO drivers and Core Audio run their callbacks on real-time threads
/// of their own.
fn callback_priority(asio: bool) -> bool {
    cfg!(any(windows, target_os = "linux")) && !asio
}

/// **RT** (a syscall on the first call per thread only): see [`callback_priority`].
fn promote_callback_thread(period: Duration) {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !DONE.with(|d| d.replace(true)) {
        crate::workers::promote_current_thread(period);
    }
}

fn build_cpal(
    renderer: RtRenderer,
    settings: &AudioSettings,
    device: &cpal::Device,
    system_buffer: bool,
    promote: bool,
    shared: Arc<AudioShared>,
) -> Result<(cpal::Stream, StreamInfo), AudioError> {
    let device = device.clone();
    let rate = renderer_rate(&renderer);
    let (config, format) = cpal_config(&device, settings, Some(rate), system_buffer)?;
    let stream = match format {
        cpal::SampleFormat::F32 => build_typed::<f32>(&device, &config, renderer, promote, shared),
        cpal::SampleFormat::F64 => build_typed::<f64>(&device, &config, renderer, promote, shared),
        cpal::SampleFormat::I16 => build_typed::<i16>(&device, &config, renderer, promote, shared),
        cpal::SampleFormat::U16 => build_typed::<u16>(&device, &config, renderer, promote, shared),
        cpal::SampleFormat::I32 => build_typed::<i32>(&device, &config, renderer, promote, shared),
        other => return Err(AudioError::Config(format!("sample format {other:?}"))),
    }?;
    stream
        .play()
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    let info = StreamInfo {
        backend: AudioBackendKind::Cpal,
        device: device.name().ok(),
        sample_rate: config.sample_rate,
        buffer_size: match config.buffer_size {
            cpal::BufferSize::Fixed(n) => n,
            cpal::BufferSize::Default => 0,
        },
        channels: config.channels,
    };
    tracing::info!(?info, "cpal output stream started");
    Ok((stream, info))
}

fn renderer_rate(r: &RtRenderer) -> u32 {
    r.sample_rate() as u32
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut renderer: RtRenderer,
    promote: bool,
    shared: Arc<AudioShared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    // The time-constraint period (macOS only, never promoted here): one buffer, or 10 ms.
    let period = match config.buffer_size {
        cpal::BufferSize::Fixed(n) => {
            Duration::from_secs_f64(f64::from(n) / f64::from(config.sample_rate.max(1)))
        }
        cpal::BufferSize::Default => Duration::from_millis(10),
    };
    let mut poisoned = false;
    let timing = shared.clone();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info| {
                enable_flush_denormals();
                if promote {
                    promote_callback_thread(period);
                }
                crate::recording::output_timing(&timing, info);
                if poisoned {
                    data.fill(T::EQUILIBRIUM);
                    return;
                }
                // Never unwind into the OS audio callback: a panic (engine/plugin bug)
                // silences the stream instead (catch_unwind is free when nothing panics).
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    renderer.render(data, channels)
                }));
                if r.is_err() {
                    poisoned = true;
                    // Reported by the controller thread, which restarts the stream.
                    timing.render_crashed.store(true, Ordering::SeqCst);
                    data.fill(T::EQUILIBRIUM);
                }
            },
            move |err| on_stream_error(&shared, &err),
            None,
        )
        .map_err(|e| AudioError::Backend(e.to_string()))
}

/// cpal's output error callback (a cpal-internal, non-RT thread). A dropout is counted;
/// `DeviceNotAvailable` (unplugged USB interface, Bluetooth headphones switched off) means
/// the stream is dead for good: flagged for the controller thread, which notifies and
/// moves to another device.
pub(crate) fn on_stream_error(shared: &AudioShared, err: &cpal::StreamError) {
    shared.xruns.fetch_add(1, Ordering::Relaxed);
    if matches!(err, cpal::StreamError::DeviceNotAvailable) {
        if !shared.device_lost.swap(true, Ordering::SeqCst) {
            tracing::warn!("audio output device lost");
        }
        shared.running.store(false, Ordering::Relaxed);
    } else {
        tracing::warn!(%err, "audio stream error");
    }
}

/// Whether `settings`' output device can be opened now (always for the null/offline
/// backends): probes a lost device before switching back to it.
pub fn output_available(settings: &AudioSettings) -> bool {
    match settings.backend {
        AudioBackendKind::Cpal => cpal_device(settings).is_ok(),
        AudioBackendKind::Null | AudioBackendKind::Offline => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue a vanished device is flagged (stream dead) and logged once; other
    /// errors only count a dropout.
    #[test]
    fn device_not_available_flags_the_device_lost() {
        let shared = AudioShared::default();
        shared.running.store(true, Ordering::Relaxed);
        let backend = cpal::StreamError::BackendSpecific {
            err: cpal::BackendSpecificError {
                description: "underrun".into(),
            },
        };
        on_stream_error(&shared, &backend);
        assert!(!shared.device_lost.load(Ordering::SeqCst));
        assert!(shared.running.load(Ordering::Relaxed));
        on_stream_error(&shared, &cpal::StreamError::DeviceNotAvailable);
        assert!(shared.device_lost.load(Ordering::SeqCst));
        assert!(!shared.running.load(Ordering::Relaxed));
        assert_eq!(shared.xruns.load(Ordering::Relaxed), 2);
    }

    fn null_settings(backend: AudioBackendKind) -> AudioSettings {
        AudioSettings {
            backend,
            max_block_size: 256,
            ..Default::default()
        }
    }

    #[test]
    fn merge_protocol_config() {
        let s = AudioSettings::default();
        let m = s
            .merged(&AudioConfig {
                backend: Some("null".into()),
                host: None,
                output_device: Some("Speakers".into()),
                input_device: None,
                sample_rate: Some(44_100),
                buffer_size: None,
            })
            .unwrap();
        assert_eq!(m.backend, AudioBackendKind::Null);
        assert_eq!(m.output_device.as_deref(), Some("Speakers"));
        assert_eq!(m.sample_rate, Some(44_100));
        assert!(
            s.merged(&AudioConfig {
                backend: Some("bogus".into()),
                host: None,
                output_device: None,
                input_device: None,
                sample_rate: None,
                buffer_size: None,
            })
            .is_err()
        );
    }

    #[test]
    fn switching_host_resets_its_device_names() {
        let config = |host: Option<&str>, output: Option<&str>, input: Option<&str>| AudioConfig {
            backend: None,
            host: host.map(Into::into),
            output_device: output.map(Into::into),
            input_device: input.map(Into::into),
            sample_rate: None,
            buffer_size: None,
        };
        let wasapi = AudioSettings {
            host: Some("WASAPI".into()),
            output_device: Some("Speakers (Interface)".into()),
            input_device: Some("Line 1/2 (Interface)".into()),
            ..Default::default()
        };
        // Another host: WASAPI names mean nothing there.
        let asio = wasapi.merged(&config(Some("ASIO"), None, None)).unwrap();
        assert_eq!(asio.output_device, None);
        assert_eq!(asio.input_device.as_deref(), Some("default"));
        // Devices sent with the host are kept.
        let m = wasapi
            .merged(&config(
                Some("ASIO"),
                Some("Interface ASIO"),
                Some("Interface ASIO"),
            ))
            .unwrap();
        assert_eq!(m.output_device.as_deref(), Some("Interface ASIO"));
        assert_eq!(m.input_device.as_deref(), Some("Interface ASIO"));
        // The same host (the dialog re-sending it) changes nothing.
        assert_eq!(
            wasapi.merged(&config(Some("WASAPI"), None, None)).unwrap(),
            wasapi
        );
        // No input stays no input; the test loopback stays.
        let silent = AudioSettings {
            input_device: None,
            ..wasapi.clone()
        };
        assert_eq!(
            silent
                .merged(&config(Some("ASIO"), None, None))
                .unwrap()
                .input_device,
            None
        );
        let looped = AudioSettings {
            input_device: Some("loopback:64".into()),
            ..wasapi
        };
        assert_eq!(
            looped
                .merged(&config(Some("ASIO"), None, None))
                .unwrap()
                .input_device
                .as_deref(),
            Some("loopback:64")
        );
    }

    #[test]
    fn device_ids_round_trip_through_the_saved_settings() {
        let s = AudioSettings {
            host: Some("ALSA".into()),
            output_device: Some("hw:CARD=USB,DEV=0".into()),
            output_device_id: Some("alsa:hw:CARD=USB,DEV=0".into()),
            input_device: Some("Line In".into()),
            input_device_id: Some("alsa:hw:CARD=USB,DEV=0".into()),
            buffer_size: Some(256),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(serde_json::from_str::<AudioSettings>(&json).unwrap(), s);
        // No id: none written (the file stays as it was before ids).
        let plain = serde_json::to_string(&AudioSettings::default()).unwrap();
        assert!(!plain.contains("device_id"), "{plain}");
    }

    /// Settings saved before ids (cpal 0.16): read by name, given the ids once the devices
    /// are listed, and found by id after a rename.
    #[test]
    fn name_only_settings_migrate_to_ids() {
        let saved = r#"{"backend":"cpal","host":null,"output_device":"Speakers",
            "input_device":"Mic","sample_rate":null,"buffer_size":null,"max_block_size":1024}"#;
        let s: AudioSettings = serde_json::from_str(saved).unwrap();
        assert_eq!(s.output_device.as_deref(), Some("Speakers"));
        assert_eq!(s.output_device_id, None);
        let listed = |name: &str, id: &str| cpal_host::Listed {
            name: Some(name.into()),
            id: Some(id.into()),
        };
        let outputs = [
            listed("Headphones", "coreaudio:hp"),
            listed("Speakers", "coreaudio:spk"),
        ];
        let (mut name, mut id) = (s.output_device.clone(), s.output_device_id.clone());
        identify(&mut name, &mut id, &outputs);
        assert_eq!(
            (name.as_deref(), id.as_deref()),
            (Some("Speakers"), Some("coreaudio:spk"))
        );
        // Renamed by the OS (or a cpal upgrade): found by id, and the name follows.
        let renamed = [listed("MacBook Pro Speakers", "coreaudio:spk")];
        identify(&mut name, &mut id, &renamed);
        assert_eq!(name.as_deref(), Some("MacBook Pro Speakers"));
        assert_eq!(id.as_deref(), Some("coreaudio:spk"));
        // Unplugged: what was saved stays.
        identify(&mut name, &mut id, &[]);
        assert_eq!(name.as_deref(), Some("MacBook Pro Speakers"));
        assert_eq!(id.as_deref(), Some("coreaudio:spk"));
        // Only real input devices get an id (the default input follows the system).
        assert!(is_input_device_name("Mic"));
        assert!(!is_input_device_name("default"));
        assert!(!is_input_device_name("loopback:64"));
    }

    #[test]
    fn choosing_another_device_forgets_the_old_id() {
        let s = AudioSettings {
            output_device: Some("Speakers".into()),
            output_device_id: Some("wasapi:spk".into()),
            input_device: Some("Mic".into()),
            input_device_id: Some("wasapi:mic".into()),
            ..Default::default()
        };
        let config = |output: Option<&str>, input: Option<&str>| AudioConfig {
            backend: None,
            host: None,
            output_device: output.map(Into::into),
            input_device: input.map(Into::into),
            sample_rate: None,
            buffer_size: Some(128),
        };
        // The same devices (or none sent): the ids stay.
        let same = s.merged(&config(Some("Speakers"), None)).unwrap();
        assert_eq!(same.output_device_id.as_deref(), Some("wasapi:spk"));
        assert_eq!(same.input_device_id.as_deref(), Some("wasapi:mic"));
        let other = s.merged(&config(Some("Headphones"), Some(""))).unwrap();
        assert_eq!(other.output_device_id, None);
        assert_eq!(other.input_device, None);
        assert_eq!(other.input_device_id, None);
    }

    #[test]
    fn buffer_size_only_change_keeps_the_default_output() {
        let s = AudioSettings {
            buffer_size: Some(256),
            ..Default::default()
        };
        let m = s
            .merged(&AudioConfig {
                backend: None,
                host: None,
                output_device: None,
                input_device: None,
                sample_rate: None,
                buffer_size: Some(1024),
            })
            .unwrap();
        // A real change (`reconfigure` restarts the stream with it); the system default
        // output stays the default.
        assert_ne!(m, s);
        assert_eq!(m.buffer_size, Some(1024));
        assert_eq!(m.output_device, None);
    }

    #[test]
    fn buffer_size_is_clamped_into_the_device_range() {
        use cpal::BufferSize::{Default as Driver, Fixed};
        use cpal::SupportedBufferSize::{Range, Unknown};
        // CoreAudio's built-in output reports 15..=4096.
        let core = Range { min: 15, max: 4096 };
        assert_eq!(pick_buffer_size(Some(512), &core, false), Fixed(512));
        assert_eq!(pick_buffer_size(Some(32), &core, false), Fixed(32));
        // Outside the range: the nearest size the device runs, not the driver default.
        assert_eq!(pick_buffer_size(Some(8192), &core, false), Fixed(4096));
        let narrow = Range {
            min: 256,
            max: 2048,
        };
        assert_eq!(pick_buffer_size(Some(64), &narrow, false), Fixed(256));
        // Nothing asked: the driver's.
        assert_eq!(pick_buffer_size(None, &core, false), Driver);
        assert_eq!(pick_buffer_size(Some(0), &core, false), Driver);
        assert_eq!(pick_buffer_size(Some(128), &Unknown, false), Fixed(128));
        // WASAPI shared mode: the system's period, whatever is asked.
        let wasapi = Range {
            min: 0,
            max: u32::MAX,
        };
        assert_eq!(pick_buffer_size(Some(128), &wasapi, true), Driver);
        // An unconstrained range elsewhere is taken as is.
        assert_eq!(pick_buffer_size(Some(128), &wasapi, false), Fixed(128));
        // A broken range doesn't produce an impossible size.
        let bad = Range { min: 512, max: 128 };
        assert_eq!(pick_buffer_size(Some(256), &bad, false), Driver);
    }

    #[test]
    fn listed_buffer_range_skips_unconstrained() {
        use cpal::SupportedBufferSize::{Range, Unknown};
        assert_eq!(
            buffer_size_range(&Range { min: 15, max: 4096 }),
            Some([15, 4096])
        );
        assert_eq!(
            buffer_size_range(&Range {
                min: 0,
                max: u32::MAX
            }),
            None
        );
        assert_eq!(buffer_size_range(&Unknown), None);
    }

    /// On a machine with an audio device (`cargo test -p ether-native -- --ignored
    /// cpal_runs_the_requested_buffer_size`): each requested size is what the callbacks
    /// get, and what the status would report.
    #[test]
    #[ignore = "needs an audio output device"]
    fn cpal_runs_the_requested_buffer_size() {
        let probe = AudioSettings::default();
        let rate = resolve(&probe).unwrap().info.sample_rate;
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: rate,
            max_block_size: 1024,
            ..Default::default()
        });
        let shared = Arc::new(AudioShared::default());
        let mut engine = Box::new(parts.engine);
        for n in [256, 64, 1024, 128] {
            let settings = AudioSettings {
                buffer_size: Some(n),
                sample_rate: Some(rate),
                ..Default::default()
            };
            let out = AudioOutput::start(engine, &settings, shared.clone())
                .map_err(|(e, _)| e)
                .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(out.info.buffer_size, n);
            assert_eq!(shared.buffer_size.load(Ordering::Relaxed), n);
            engine = out.stop().expect("engine back");
        }
        // None asked: the host's default (ALSA: `ALSA_DEFAULT_BUFFER`, not the device period).
        let settings = AudioSettings {
            sample_rate: Some(rate),
            ..Default::default()
        };
        let out = AudioOutput::start(engine, &settings, shared.clone())
            .map_err(|(e, _)| e)
            .unwrap();
        let host = cpal_host::host(None).unwrap();
        if let Some(n) = cpal_host::default_buffer_frames(&host) {
            assert_eq!(out.info.buffer_size, n);
        }
        println!(
            "default buffer: {} frames",
            shared.buffer_size.load(Ordering::Relaxed)
        );
        out.stop().expect("engine back");
    }

    /// On a machine with audio devices (`cargo test -p ether-native -- --ignored
    /// lists_the_useful_devices --nocapture`): the default list is the full one minus what
    /// it reports hidden, and a listed device saved by name gets its id.
    #[test]
    #[ignore = "needs audio devices"]
    fn lists_the_useful_devices() {
        let settings = AudioSettings::default();
        let short = list_devices(&settings, false);
        let full = list_devices(&settings, true);
        let names = |l: &AudioDeviceList| l.outputs.iter().map(|d| d.name.clone()).collect();
        let (short_names, full_names): (Vec<String>, Vec<String>) = (names(&short), names(&full));
        println!(
            "shown: {short_names:?}\nall: {full_names:?}\nhidden: {}",
            short.hidden_devices
        );
        assert_eq!(full.hidden_devices, 0);
        assert_eq!(
            short.outputs.len() + short.inputs.len() + short.hidden_devices as usize,
            full.outputs.len() + full.inputs.len()
        );
        let Some(name) = short_names.first() else {
            return;
        };
        let saved = AudioSettings {
            output_device: Some(name.clone()),
            ..settings
        };
        let migrated = saved.with_device_ids().expect("an id for a listed device");
        println!("{name} -> {:?}", migrated.output_device_id);
        assert!(migrated.output_device_id.is_some());
        assert!(migrated.with_device_ids().is_none(), "migrated once");
    }

    #[test]
    fn asio_not_found_says_what_to_check() {
        let e = cpal_host::not_found(true, Some("Interface ASIO"), "output").to_string();
        assert!(
            e.contains("Interface ASIO") && e.contains("in use by another application"),
            "{e}"
        );
        assert_eq!(
            cpal_host::not_found(false, None, "output").to_string(),
            "audio device not found: default output"
        );
    }

    #[test]
    fn null_backend_paces_and_returns_engine() {
        let settings = null_settings(AudioBackendKind::Null);
        let info = resolve(&settings).unwrap().info;
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.buffer_size, 256);
        let parts = ether_core::create(ether_core::EngineConfig {
            sample_rate: info.sample_rate,
            max_block_size: settings.max_block_size,
            ..Default::default()
        });
        let shared = Arc::new(AudioShared::default());
        let out = AudioOutput::start(Box::new(parts.engine), &settings, shared.clone())
            .map_err(|(e, _)| e)
            .unwrap();
        assert!(shared.running.load(Ordering::Relaxed));
        std::thread::sleep(Duration::from_millis(200));
        let frames = shared.frames.load(Ordering::Relaxed);
        // ~9600 frames in 200 ms at real-time pace; generous bounds for loaded CI boxes.
        assert!(frames > 2_000 && frames < 20_000, "{frames}");
        let engine = out.stop().expect("engine back");
        assert_eq!(engine.config().sample_rate, 48_000);
        assert!(!shared.running.load(Ordering::Relaxed));
    }

    #[test]
    fn offline_backend_runs_faster_than_real_time() {
        let settings = null_settings(AudioBackendKind::Offline);
        let parts = ether_core::create(ether_core::EngineConfig {
            max_block_size: settings.max_block_size,
            ..Default::default()
        });
        let shared = Arc::new(AudioShared::default());
        let out = AudioOutput::start(Box::new(parts.engine), &settings, shared.clone())
            .map_err(|(e, _)| e)
            .unwrap();
        // Faster than real time over some 100 ms window: more than 100 ms of audio (4800
        // frames at 48 kHz). The paced Null backend can't (it resyncs instead of catching up
        // with a burst). Sliding windows sampled every 10 ms for up to 20 s: on a loaded
        // machine the thread gets little CPU, and a descheduled test thread sees a burst.
        let mut samples = std::collections::VecDeque::new();
        let mut best = 0;
        for _ in 0..2000 {
            samples.push_back(shared.frames.load(Ordering::Relaxed));
            if samples.len() > 10 {
                let (now, then) = (*samples.back().unwrap(), samples.pop_front().unwrap());
                best = best.max(now - then);
            }
            if best > 48_000 / 10 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(best > 48_000 / 10, "at most {best} frames in 100 ms");
        assert!(out.stop().is_some());
    }
}
