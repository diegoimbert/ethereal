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
    /// From `ETHER_AUDIO` (`cpal` | `null` | `offline`), default `Cpal`.
    pub fn from_env() -> Self {
        Self::parse(std::env::var("ETHER_AUDIO").ok().as_deref()).unwrap_or(Self::Cpal)
    }

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
            input_device: None,
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
        if c.host.is_some() {
            s.host = c.host.clone();
        }
        if c.output_device.is_some() {
            s.output_device = c.output_device.clone();
        }
        if c.sample_rate.is_some() {
            s.sample_rate = c.sample_rate;
        }
        if c.buffer_size.is_some() {
            s.buffer_size = c.buffer_size;
        }
        Ok(s)
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
            let (_, device) = cpal_device(settings)?;
            let (config, _) = cpal_config(&device, settings, None)?;
            Ok(ResolvedConfig {
                info: StreamInfo {
                    backend: AudioBackendKind::Cpal,
                    device: device.name().ok(),
                    sample_rate: config.sample_rate.0,
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

fn cpal_host(name: Option<&str>) -> Result<cpal::Host, AudioError> {
    let Some(name) = name else {
        return Ok(cpal::default_host());
    };
    let id = cpal::available_hosts()
        .into_iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))
        .ok_or_else(|| AudioError::DeviceNotFound(format!("audio host {name}")))?;
    cpal::host_from_id(id).map_err(|e| AudioError::Backend(e.to_string()))
}

fn cpal_device(settings: &AudioSettings) -> Result<(cpal::Host, cpal::Device), AudioError> {
    let host = cpal_host(settings.host.as_deref())?;
    let device = match &settings.output_device {
        Some(name) => host
            .output_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .find(|d| d.name().is_ok_and(|n| &n == name))
            .ok_or_else(|| AudioError::DeviceNotFound(name.clone()))?,
        None => host
            .default_output_device()
            .ok_or_else(|| AudioError::DeviceNotFound("default output".into()))?,
    };
    Ok((host, device))
}

/// Pick the stream config: the requested (or forced) sample rate if the device supports
/// it in its default sample format, else the device default; the requested buffer size if
/// within the device range.
fn cpal_config(
    device: &cpal::Device,
    settings: &AudioSettings,
    force_rate: Option<u32>,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), AudioError> {
    let default = device
        .default_output_config()
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    let format = default.sample_format();
    let wanted = force_rate.or(settings.sample_rate);
    let supported = match wanted {
        Some(rate) if rate != default.sample_rate().0 => device
            .supported_output_configs()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .filter(|c| c.sample_format() == format && c.channels() == default.channels())
            .find_map(|c| c.try_with_sample_rate(cpal::SampleRate(rate)))
            .or_else(|| {
                device.supported_output_configs().ok()?.find_map(|c| {
                    c.try_with_sample_rate(cpal::SampleRate(rate))
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
    let buffer_size = match (settings.buffer_size, supported.buffer_size()) {
        (Some(n), cpal::SupportedBufferSize::Range { min, max }) if n >= *min && n <= *max => {
            cpal::BufferSize::Fixed(n)
        }
        (Some(n), cpal::SupportedBufferSize::Unknown) => cpal::BufferSize::Fixed(n),
        _ => cpal::BufferSize::Default,
    };
    let format = supported.sample_format();
    if !is_supported_format(format) {
        return Err(AudioError::Config(format!("sample format {format:?}")));
    }
    let mut config = supported.config();
    config.buffer_size = buffer_size;
    Ok((config, format))
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

/// Enumerate cpal hosts and output devices (plus the null/offline backends).
pub fn list_devices(current: &AudioSettings) -> AudioDeviceList {
    let mut outputs = Vec::new();
    if let Ok(host) = cpal_host(current.host.as_deref()) {
        let default_name = host.default_output_device().and_then(|d| d.name().ok());
        if let Ok(devices) = host.output_devices() {
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
                                    c.min_sample_rate().0 <= *r && *r <= c.max_sample_rate().0
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
                    name,
                });
            }
        }
    }
    AudioDeviceList {
        backends: vec!["cpal".into(), "null".into(), "offline".into()],
        hosts: cpal::available_hosts()
            .into_iter()
            .map(|h| h.name().to_string())
            .collect(),
        outputs,
        inputs: Vec::new(),
        current: current.to_protocol(),
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
            let stream = match build_cpal(renderer, &settings, shared.clone()) {
                Ok((stream, info)) => {
                    let _ = ready_tx.send(Ok(info));
                    stream
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
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

fn build_cpal(
    renderer: RtRenderer,
    settings: &AudioSettings,
    shared: Arc<AudioShared>,
) -> Result<(cpal::Stream, StreamInfo), AudioError> {
    let (_host, device) = cpal_device(settings)?;
    let rate = renderer_rate(&renderer);
    let (config, format) = cpal_config(&device, settings, Some(rate))?;
    let stream = match format {
        cpal::SampleFormat::F32 => build_typed::<f32>(&device, &config, renderer, shared),
        cpal::SampleFormat::F64 => build_typed::<f64>(&device, &config, renderer, shared),
        cpal::SampleFormat::I16 => build_typed::<i16>(&device, &config, renderer, shared),
        cpal::SampleFormat::U16 => build_typed::<u16>(&device, &config, renderer, shared),
        cpal::SampleFormat::I32 => build_typed::<i32>(&device, &config, renderer, shared),
        other => return Err(AudioError::Config(format!("sample format {other:?}"))),
    }?;
    stream
        .play()
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    let info = StreamInfo {
        backend: AudioBackendKind::Cpal,
        device: device.name().ok(),
        sample_rate: config.sample_rate.0,
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
    shared: Arc<AudioShared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _info| {
                enable_flush_denormals();
                renderer.render(data, channels);
            },
            move |err| {
                // Runs on a cpal-internal (non-RT) thread.
                shared.xruns.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(%err, "audio stream error");
            },
            None,
        )
        .map_err(|e| AudioError::Backend(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::thread::sleep(Duration::from_millis(100));
        let frames = shared.frames.load(Ordering::Relaxed);
        assert!(frames > 48_000 / 10, "{frames}");
        assert!(out.stop().is_some());
    }
}
