//! The cpal input stream: device lookup, format conversion, the RT input callback.

use std::sync::atomic::Ordering;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use ether_core::protocol::engine::AudioDeviceInfo;
use ether_core::recording::rtrb::Producer;

use super::{INPUT_CHANNELS, set_input_channels, take_pending_input};
use crate::audio::AudioSettings;
use crate::rt::AudioShared;

fn host(name: Option<&str>) -> Option<cpal::Host> {
    let Some(name) = name else {
        return Some(cpal::default_host());
    };
    let id = cpal::available_hosts()
        .into_iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))?;
    cpal::host_from_id(id).ok()
}

fn device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
    if name == "default" {
        return host.default_input_device();
    }
    host.input_devices()
        .ok()?
        .find(|d| d.name().is_ok_and(|n| n == name))
}

/// Input devices of the configured cpal host (`AudioDeviceList::inputs`).
pub fn list_input_devices(settings: &AudioSettings) -> Vec<AudioDeviceInfo> {
    let Some(host) = host(settings.host.as_deref()) else {
        return Vec::new();
    };
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|d| {
            let name = d.name().ok()?;
            let config = d.default_input_config().ok()?;
            let mut rates: Vec<u32> = d
                .supported_input_configs()
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
            Some(AudioDeviceInfo {
                is_default: default_name.as_deref() == Some(name.as_str()),
                channels: config.channels(),
                sample_rates: rates,
                name,
            })
        })
        .collect()
}

/// Open the configured input device at the engine rate, feeding the ring the current
/// `InputFeed` created. Called on the cpal control thread (streams are not `Send`); the
/// stream runs until dropped. Failures are logged: the engine then gets silence.
pub(crate) fn open_input(
    settings: &AudioSettings,
    shared: &std::sync::Arc<AudioShared>,
) -> Option<cpal::Stream> {
    let (producer, host_name, device_name) = take_pending_input(shared)?;
    let rate = shared.recording.sample_rate.load(Ordering::Relaxed);
    let result = (|| -> Result<cpal::Stream, String> {
        let host = host(host_name.as_deref().or(settings.host.as_deref()))
            .ok_or("audio host not found")?;
        let name = device_name.ok_or("no input device")?;
        let device =
            device(&host, &name).ok_or_else(|| format!("input device {name} not found"))?;
        let default = device.default_input_config().map_err(|e| e.to_string())?;
        let supported = if default.sample_rate().0 == rate {
            default
        } else {
            device
                .supported_input_configs()
                .map_err(|e| e.to_string())?
                .find_map(|c| c.try_with_sample_rate(cpal::SampleRate(rate)))
                .ok_or_else(|| format!("input device {name} does not support {rate} Hz"))?
        };
        let format = supported.sample_format();
        let config = supported.config();
        set_input_channels(shared, config.channels);
        let s = shared.clone();
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, producer, s),
            cpal::SampleFormat::F64 => build::<f64>(&device, &config, producer, s),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, producer, s),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, producer, s),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, producer, s),
            other => return Err(format!("input sample format {other:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        tracing::info!(device = %name, rate, channels = config.channels, "cpal input stream started");
        Ok(stream)
    })();
    match result {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "audio input unavailable");
            None
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut ring: Producer<f32>,
    shared: std::sync::Arc<AudioShared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let rate = f64::from(config.sample_rate.0);
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                // RT: only the ring and atomics.
                let ts = info.timestamp();
                if let Some(d) = ts.callback.duration_since(&ts.capture) {
                    shared
                        .recording
                        .input_latency
                        .store((d.as_secs_f64() * rate).round() as u32, Ordering::Relaxed);
                }
                let frames = (data.len() / channels).min(ring.slots() / INPUT_CHANNELS);
                let Ok(mut chunk) = ring.write_chunk(frames * INPUT_CHANNELS) else {
                    return;
                };
                let (a, b) = chunk.as_mut_slices();
                let mut i = 0;
                for frame in data.chunks_exact(channels).take(frames) {
                    for ch in 0..INPUT_CHANNELS {
                        let s = frame.get(ch).map_or(0.0, |s| s.to_sample::<f32>());
                        if i < a.len() {
                            a[i] = s;
                        } else {
                            b[i - a.len()] = s;
                        }
                        i += 1;
                    }
                }
                chunk.commit_all();
            },
            |err| tracing::warn!(%err, "audio input stream error"),
            None,
        )
        .map_err(|e| e.to_string())
}
