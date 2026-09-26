//! Media loading for the native host (never on the audio thread).
//!
//! # Decision: whole-file decode up front, no disk streaming in v0.1
//!
//! `ether-media` decodes whole files from bytes in memory, and `ether_core::AudioSource`
//! only requires RT-safe reads. Native v0.1 decodes each audio file **completely** when it
//! is imported or when a project is opened, resamples it to the engine rate, and serves it
//! from memory (`ether_media::InMemorySource`). Rationale:
//! - projects are self-contained and typical clips are short (loops, one-shots, stems of a
//!   few minutes): 5 min stereo at 48 kHz is ~115 MB of `f32`, acceptable on desktop;
//! - no RT-side prefetch ring, no underruns, no seeking latency when looping/scrubbing,
//!   and warped playback (Signalsmith) reads arbitrary positions;
//! - symphonia decoding is fast (a 5 min file decodes in well under a second).
//!
//! Streaming can be added later without touching the engine or the controller: a
//! `StreamingSource: AudioSource` whose `read` serves from a lock-free prefetch ring filled
//! by a disk thread (`prefetch_hint` tells it where playback is heading), created by the
//! bridge's `load_media` for files above a size threshold.
//!
//! # Pipeline ([`prepare`])
//! 1. decode the file bytes (source sample rate);
//! 2. build the peak mipmap **from the source-rate audio** (peak requests are in source
//!    frames, and peaks must not depend on the engine rate);
//! 3. resample to the engine rate (rubato);
//! 4. publish to the engine as an `AudioSource` (`EngineHandle::add_source`, done by
//!    [`crate::bridge::NativeBridge::load_media`]).
//!
//! Steps 1-3 run on the calling thread, which is the controller thread (the controller
//! decodes imports) or a [`MediaWorker`] thread; never the audio thread.

use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};
use ether_media::{DecodedAudio, MediaError, PeakMipmap};

/// Decoded media ready for the engine.
#[derive(Debug, Clone)]
pub struct PreparedMedia {
    /// Peaks at the source sample rate.
    pub peaks: Arc<PeakMipmap>,
    /// Source sample rate and frame count (what `MediaRef` records).
    pub source_rate: u32,
    pub source_frames: u64,
    /// Audio resampled to the engine rate.
    pub audio: Arc<DecodedAudio>,
}

/// Decode, build peaks from the source-rate audio, then resample to `engine_rate`.
pub fn prepare(
    bytes: Vec<u8>,
    extension: Option<&str>,
    engine_rate: u32,
) -> Result<PreparedMedia, MediaError> {
    let decoded = ether_media::decode_owned(bytes, extension)?;
    let peaks = Arc::new(PeakMipmap::build(&decoded));
    let source_rate = decoded.sample_rate;
    let source_frames = decoded.frames() as u64;
    let audio = Arc::new(to_engine_rate(decoded, engine_rate)?);
    Ok(PreparedMedia {
        peaks,
        source_rate,
        source_frames,
        audio,
    })
}

/// Resample if needed (no copy when the rate already matches).
pub fn to_engine_rate(audio: DecodedAudio, engine_rate: u32) -> Result<DecodedAudio, MediaError> {
    if audio.sample_rate == engine_rate || audio.frames() == 0 {
        let mut audio = audio;
        audio.sample_rate = engine_rate;
        return Ok(audio);
    }
    ether_media::resample(&audio, engine_rate)
}

/// A job for the [`MediaWorker`].
pub struct MediaJob<T> {
    pub bytes: Vec<u8>,
    pub extension: Option<String>,
    /// Caller data returned with the result (e.g. the `MediaId`).
    pub tag: T,
}

/// Background thread running [`prepare`] so large files don't stall the caller. Results
/// are collected with [`MediaWorker::try_recv`] (e.g. from the controller tick) and then
/// published to the engine.
pub struct MediaWorker<T: Send + 'static> {
    jobs: Option<Sender<MediaJob<T>>>,
    results: Receiver<(T, Result<PreparedMedia, MediaError>)>,
    thread: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> MediaWorker<T> {
    pub fn new(engine_rate: u32) -> Self {
        let (jobs_tx, jobs_rx) = unbounded::<MediaJob<T>>();
        let (res_tx, res_rx) = unbounded();
        let thread = std::thread::Builder::new()
            .name("ether-media".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let r = prepare(job.bytes, job.extension.as_deref(), engine_rate);
                    if res_tx.send((job.tag, r)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn media thread");
        Self {
            jobs: Some(jobs_tx),
            results: res_rx,
            thread: Some(thread),
        }
    }

    pub fn submit(&self, job: MediaJob<T>) {
        if let Some(tx) = &self.jobs {
            let _ = tx.send(job);
        }
    }

    pub fn try_recv(&self) -> Option<(T, Result<PreparedMedia, MediaError>)> {
        self.results.try_recv().ok()
    }

    /// Blocking receive with a timeout (tests).
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Option<(T, Result<PreparedMedia, MediaError>)> {
        self.results.recv_timeout(timeout).ok()
    }
}

impl<T: Send + 'static> Drop for MediaWorker<T> {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Minimal 16-bit PCM WAV writer (tests and fixtures).
pub fn wav_bytes(sample_rate: u32, channels: &[Vec<f32>]) -> Vec<u8> {
    let nch = channels.len() as u16;
    let frames = channels.first().map_or(0, Vec::len);
    let data_len = frames as u32 * nch as u32 * 2;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&nch.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * nch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(nch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for ch in channels {
            let s = (ch[i].clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            b.extend_from_slice(&s.to_le_bytes());
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sine(rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn prepare_builds_source_rate_peaks_and_resamples() {
        let wav = wav_bytes(44_100, &[sine(44_100, 44_100), sine(44_100, 44_100)]);
        let p = prepare(wav, Some("wav"), 48_000).unwrap();
        assert_eq!(p.source_rate, 44_100);
        assert_eq!(p.source_frames, 44_100);
        assert_eq!(p.peaks.frames(), 44_100);
        assert_eq!(p.peaks.channels(), 2);
        assert_eq!(p.audio.sample_rate, 48_000);
        let expected = ether_media::resampled_len(44_100, 44_100, 48_000);
        assert_eq!(p.audio.frames(), expected);
    }

    #[test]
    fn same_rate_is_not_resampled() {
        let wav = wav_bytes(48_000, &[sine(48_000, 1000)]);
        let p = prepare(wav, Some("wav"), 48_000).unwrap();
        assert_eq!(p.audio.frames(), 1000);
    }

    #[test]
    fn worker_prepares_off_thread() {
        let w = MediaWorker::new(48_000);
        w.submit(MediaJob {
            bytes: wav_bytes(48_000, &[sine(48_000, 480)]),
            extension: Some("wav".into()),
            tag: 7u32,
        });
        w.submit(MediaJob {
            bytes: b"garbage".to_vec(),
            extension: Some("wav".into()),
            tag: 8u32,
        });
        let (tag, r) = w.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(tag, 7);
        assert_eq!(r.unwrap().audio.frames(), 480);
        let (tag, r) = w.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(tag, 8);
        assert!(r.is_err());
    }
}
