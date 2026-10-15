//! The cpal input stream: device lookup, format conversion, the RT input callback.

use std::sync::atomic::Ordering;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use ether_core::protocol::engine::AudioDeviceInfo;
use ether_core::recording::rtrb::Producer;

use super::{MAX_INPUT_CHANNELS, set_input_channels, take_pending_input};
use crate::audio::AudioSettings;
use crate::cpal_host;
use crate::rt::AudioShared;

fn device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
    if name == "default" {
        return if cpal_host::is_asio(host) {
            // No default ASIO device (and asking for one loads drivers): the first with inputs.
            host.input_devices()
                .ok()?
                .find(|d| d.default_input_config().is_ok())
        } else {
            host.default_input_device()
        };
    }
    host.input_devices()
        .ok()?
        .find(|d| d.name().is_ok_and(|n| n == name))
}

/// Input devices of the configured cpal host (`AudioDeviceList::inputs`).
pub fn list_input_devices(settings: &AudioSettings) -> Vec<AudioDeviceInfo> {
    let Ok(host) = cpal_host::host(settings.host.as_deref()) else {
        return Vec::new();
    };
    let asio = cpal_host::is_asio(&host);
    let default_name = if asio {
        None
    } else {
        host.default_input_device().and_then(|d| d.name().ok())
    };
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut list: Vec<AudioDeviceInfo> = devices
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
            // Every input channel the device can open (the stream opens them all).
            let channels = d
                .supported_input_configs()
                .ok()
                .and_then(|cs| cs.map(|c| c.channels()).max())
                .unwrap_or(0)
                .max(config.channels());
            Some(AudioDeviceInfo {
                is_default: default_name.as_deref() == Some(name.as_str()),
                channels,
                sample_rates: rates,
                buffer_size_range: None,
                name,
            })
        })
        .collect();
    if asio {
        crate::audio::push_unopened_asio_drivers(&mut list);
    }
    list
}

/// ASIO duplex: the input opens on the output's driver, after it, with its buffer size
/// (`audio::start_cpal`).
pub(crate) struct AsioDuplex<'a> {
    pub output: &'a cpal::Device,
    /// The output stream's buffer size; `None` = the driver's default (as the output used).
    pub buffer_frames: Option<u32>,
}

/// Open the configured input device at the engine rate, feeding the ring the current
/// `InputFeed` created. Called on the cpal control thread (streams are not `Send`); the
/// stream runs until dropped. Failures are logged: the engine then gets silence.
///
/// On ASIO (`asio`) the input is the driver the output plays through, opened after it with
/// its buffer size: only one driver can be loaded, and cpal pairs both streams on the same
/// device.
pub(crate) fn open_input(
    settings: &AudioSettings,
    shared: &std::sync::Arc<AudioShared>,
    asio: Option<AsioDuplex<'_>>,
) -> Option<cpal::Stream> {
    let asio_output = asio.as_ref().map(|a| a.output);
    let (producer, host_name, device_name) = take_pending_input(shared)?;
    let rate = shared.recording.sample_rate.load(Ordering::Relaxed);
    let result = (|| -> Result<cpal::Stream, String> {
        let name = device_name.ok_or("no input device")?;
        let device = match asio_output {
            Some(output) => {
                let driver = output.name().map_err(|e| e.to_string())?;
                if name != "default" && name != driver {
                    return Err(format!(
                        "{name} is a different ASIO driver than the output ({driver}): ASIO uses one driver for both"
                    ));
                }
                output.clone()
            }
            None => {
                let host = cpal_host::host(host_name.as_deref().or(settings.host.as_deref()))
                    .map_err(|e| e.to_string())?;
                device(&host, &name).ok_or_else(|| format!("input device {name} not found"))?
            }
        };
        let supported = input_config(&device, rate)?
            .ok_or_else(|| format!("input device {name} does not support {rate} Hz"))?;
        let format = supported.sample_format();
        let mut config = supported.config();
        // ASIO: the output's buffer size (both streams share the driver's buffers).
        if let Some(n) = asio.as_ref().and_then(|a| a.buffer_frames) {
            config.buffer_size = cpal::BufferSize::Fixed(n);
        }
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
            shared
                .recording
                .set_input_error(format!("Audio input unavailable: {e}"));
            None
        }
    }
}

/// The input configuration at `rate` with the most channels (a multi-channel interface
/// opens every input, tracks pick theirs), preferring the default's sample format among
/// equals; the default configuration if it has as many.
fn input_config(
    device: &cpal::Device,
    rate: u32,
) -> Result<Option<cpal::SupportedStreamConfig>, String> {
    let default = device.default_input_config().ok();
    let configs = match device.supported_input_configs() {
        Ok(cs) => cs.collect::<Vec<_>>(),
        Err(_) if default.is_some() => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let best = configs
        .into_iter()
        .filter_map(|c| c.try_with_sample_rate(cpal::SampleRate(rate)))
        .max_by_key(|c| {
            (
                c.channels(),
                default
                    .as_ref()
                    .is_some_and(|d| d.sample_format() == c.sample_format()),
            )
        });
    Ok(match (default, best) {
        (Some(d), best)
            if d.sample_rate().0 == rate
                && best.as_ref().is_none_or(|b| b.channels() <= d.channels()) =>
        {
            Some(d)
        }
        (_, best) => best,
    })
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
    // Frames in the ring carry the device's first `MAX_INPUT_CHANNELS` channels.
    let kept = channels.min(MAX_INPUT_CHANNELS);
    let rate = f64::from(config.sample_rate.0);
    let errors = shared.clone();
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
                let frames = (data.len() / channels).min(ring.slots() / kept);
                let Ok(mut chunk) = ring.write_chunk(frames * kept) else {
                    return;
                };
                let (a, b) = chunk.as_mut_slices();
                let mut i = 0;
                for frame in data.chunks_exact(channels).take(frames) {
                    for s in &frame[..kept] {
                        let s = s.to_sample::<f32>();
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
            move |err| {
                // Non-RT (cpal error thread): e.g. the device was unplugged.
                tracing::warn!(%err, "audio input stream error");
                errors
                    .recording
                    .set_input_error(format!("Audio input failed: {err}"));
            },
            None,
        )
        .map_err(|e| e.to_string())
}
