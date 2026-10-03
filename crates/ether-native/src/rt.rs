//! Real-time side of the host: the render callback state shared by every audio backend,
//! and the denormal guard.
//!
//! Everything reachable from [`RtRenderer::render`] follows the real-time rules of
//! `docs/ARCHITECTURE.md`: no allocation, no locks, no I/O, bounded work. Buffers are
//! allocated in [`RtRenderer::new`]; the engine is handed back to the host (non-RT) when the
//! renderer is dropped, which happens on the thread that stops the stream.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use cpal::{FromSample, Sample};
use crossbeam_channel::Sender;
use ether_core::Engine;

/// Enable flush-to-zero / denormals-are-zero on the **current thread**.
///
/// Denormal floats (tiny values in decaying filters/reverb tails) are up to ~100x slower on
/// most CPUs and can blow the audio deadline. Built-in devices guard their own decays, but
/// plugins and future code may not, so every thread that runs `Engine::process` calls this
/// (the cpal callback on every invocation, since some backends may change threads; the
/// null/offline thread once at start). The FP control register is per thread, so this does
/// not affect the rest of the process.
///
/// - x86 / x86_64: sets MXCSR bits FTZ (15) and DAZ (6).
/// - aarch64: sets FPCR bit FZ (24).
/// - other targets: no-op.
#[inline]
pub fn enable_flush_denormals() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: reads/writes only the MXCSR control/status register of this thread; FTZ/DAZ
    // only change how denormals are treated (flushed to zero), never trap.
    unsafe {
        let mut csr: u32 = 0;
        std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack, preserves_flags));
        csr |= (1 << 15) | (1 << 6);
        std::arch::asm!("ldmxcsr [{}]", in(reg) &csr, options(nostack, preserves_flags, readonly));
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: reads/writes only this thread's FPCR; FZ only flushes denormals to zero.
    unsafe {
        let mut fpcr: u64;
        std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags));
        fpcr |= 1 << 24;
        std::arch::asm!("msr fpcr, {}", in(reg) fpcr, options(nomem, nostack, preserves_flags));
    }
}

/// Whether flush-to-zero is active on the current thread (for tests/diagnostics).
pub fn flush_denormals_enabled() -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let mut csr: u32 = 0;
        // SAFETY: reads this thread's MXCSR.
        unsafe {
            std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack, preserves_flags));
        }
        csr & (1 << 15) != 0
    }
    #[cfg(target_arch = "aarch64")]
    {
        let fpcr: u64;
        // SAFETY: reads this thread's FPCR.
        unsafe {
            std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags));
        }
        fpcr & (1 << 24) != 0
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

/// Hardware output channels the engine renders (more device channels are silent).
pub const MAX_OUTPUT_CHANNELS: usize = 32;

/// Lock-free status the audio thread publishes for the host (engine status, meters).
#[derive(Debug, Default)]
pub struct AudioShared {
    pub running: AtomicBool,
    /// Dropouts reported by the backend (cpal stream errors, null-thread overruns).
    pub xruns: AtomicU32,
    /// Smoothed DSP load 0..1 (`f32` bits): callback time / buffer duration.
    cpu_load: AtomicU32,
    /// Last device buffer size in frames.
    pub buffer_size: AtomicU32,
    /// Frames rendered since start.
    pub frames: AtomicU64,
    /// Input capture / recording state ([`crate::recording`]).
    pub recording: crate::recording::RecordingShared,
    /// Output channels of the running stream (hardware sends of external devices may use
    /// all of them; the master bus is on the first two).
    pub output_channels: AtomicU32,
    /// Wall clock ↔ engine sample time at the start of the last rendered chunk (hardware MIDI
    /// out scheduling, `external-instrument`).
    pub clock: EngineClock,
}

/// Single-writer seqlock: the wall time (ns since the clock's creation) at which the engine
/// started rendering sample `sample`.
#[derive(Debug)]
pub struct EngineClock {
    epoch: Instant,
    seq: AtomicU64,
    ns: AtomicU64,
    sample: AtomicU64,
}

impl Default for EngineClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            seq: AtomicU64::new(0),
            ns: AtomicU64::new(0),
            sample: AtomicU64::new(0),
        }
    }
}

impl EngineClock {
    /// Nanoseconds since the clock's creation.
    pub fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// **RT.** The engine is about to render `sample` now.
    pub fn publish(&self, sample: u64) {
        let ns = self.now_ns();
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        self.ns.store(ns, Ordering::Relaxed);
        self.sample.store(sample, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    /// `(ns, sample)` of the last publish (`(0, 0)` before the first).
    pub fn read(&self) -> (u64, u64) {
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let ns = self.ns.load(Ordering::Relaxed);
            let sample = self.sample.load(Ordering::Relaxed);
            std::sync::atomic::fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return (ns, sample);
            }
        }
    }

    /// The engine sample being rendered now (extrapolated at `sample_rate`), or `None`
    /// before the first block.
    pub fn sample_now(&self, sample_rate: f64) -> Option<f64> {
        let (ns, sample) = self.read();
        if ns == 0 && sample == 0 {
            return None;
        }
        let elapsed = self.now_ns().saturating_sub(ns) as f64 * 1e-9;
        Some(sample as f64 + elapsed * sample_rate)
    }
}

impl AudioShared {
    pub fn cpu_load(&self) -> f32 {
        f32::from_bits(self.cpu_load.load(Ordering::Relaxed))
    }
}

/// Callback state owned by the audio thread.
pub struct RtRenderer {
    engine: Option<Box<Engine>>,
    /// Planar output scratch ([`MAX_OUTPUT_CHANNELS`] × max block; the master bus is on the
    /// first two, external devices' hardware sends may use any).
    outs: Vec<Vec<f32>>,
    /// Engine input: hardware input, loopback or silence ([`crate::recording`]).
    input: crate::recording::InputFeed,
    max_block: usize,
    sample_rate: f64,
    shared: Arc<AudioShared>,
    give_back: Sender<Box<Engine>>,
}

impl RtRenderer {
    /// Non-RT. `give_back` receives the engine when the renderer is dropped.
    pub fn new(
        engine: Box<Engine>,
        shared: Arc<AudioShared>,
        give_back: Sender<Box<Engine>>,
    ) -> Self {
        let max_block = engine.config().max_block_size.max(1);
        let sample_rate = engine.config().sample_rate as f64;
        Self {
            engine: Some(engine),
            outs: (0..MAX_OUTPUT_CHANNELS)
                .map(|_| vec![0.0; max_block])
                .collect(),
            input: crate::recording::InputFeed::new(&shared, max_block, sample_rate),
            max_block,
            sample_rate,
            shared,
            give_back,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// **RT.** Fill an interleaved device buffer with `channels` channels. The buffer may be
    /// larger than the engine's max block: it is rendered in chunks of at most
    /// `max_block_size` frames. The engine renders up to [`MAX_OUTPUT_CHANNELS`] channels:
    /// the master bus on the first two, external devices' hardware sends on theirs (extra
    /// channels are silent; mono devices get the L/R average).
    pub fn render<T>(&mut self, out: &mut [T], channels: usize)
    where
        T: Sample + FromSample<f32>,
    {
        let start = Instant::now();
        let channels = channels.max(1);
        let total = out.len() / channels;
        let Some(engine) = self.engine.as_mut() else {
            out.fill(T::EQUILIBRIUM);
            return;
        };
        let n_out = channels.clamp(2, MAX_OUTPUT_CHANNELS);
        self.shared
            .output_channels
            .store(channels as u32, Ordering::Relaxed);
        let mut done = 0;
        while done < total {
            let n = (total - done).min(self.max_block);
            self.shared.clock.publish(engine.sample_time());
            {
                let inputs = self.input.read(n);
                let mut outputs: [&mut [f32]; MAX_OUTPUT_CHANNELS] = {
                    let mut it = self.outs.iter_mut();
                    std::array::from_fn(|_| match it.next() {
                        Some(v) => &mut v[..n],
                        None => &mut [],
                    })
                };
                process_guarded(engine, &inputs, &mut outputs[..n_out], n);
            }
            self.input
                .after_process(&self.outs[0][..n], &self.outs[1][..n]);
            let frames = &mut out[done * channels..(done + n) * channels];
            for (i, frame) in frames.chunks_exact_mut(channels).enumerate() {
                if channels == 1 {
                    frame[0] = T::from_sample((self.outs[0][i] + self.outs[1][i]) * 0.5);
                } else {
                    for (c, s) in frame.iter_mut().enumerate() {
                        *s = match self.outs.get(c) {
                            Some(o) => T::from_sample(o[i]),
                            None => T::EQUILIBRIUM,
                        };
                    }
                }
            }
            done += n;
        }
        self.shared
            .frames
            .fetch_add(total as u64, Ordering::Relaxed);
        self.shared
            .buffer_size
            .store(total as u32, Ordering::Relaxed);
        if total > 0 {
            let budget = total as f64 / self.sample_rate;
            let load = (start.elapsed().as_secs_f64() / budget) as f32;
            let prev = self.shared.cpu_load();
            let smoothed = prev + (load - prev) * 0.1;
            self.shared
                .cpu_load
                .store(smoothed.to_bits(), Ordering::Relaxed);
        }
    }
}

/// `Engine::process`, wrapped in `assert_no_alloc` in debug builds (effective when the
/// binary installs `assert_no_alloc::AllocDisabler` as its global allocator; the desktop
/// app does in debug builds, with the `warn_debug` feature so a violation is logged, not
/// fatal).
#[inline]
fn process_guarded(
    engine: &mut Engine,
    inputs: &[&[f32]],
    outputs: &mut [&mut [f32]],
    frames: usize,
) {
    #[cfg(debug_assertions)]
    assert_no_alloc::assert_no_alloc(|| engine.process(inputs, outputs, frames));
    #[cfg(not(debug_assertions))]
    engine.process(inputs, outputs, frames);
}

impl Drop for RtRenderer {
    /// Runs where the stream is torn down (never on the audio thread while it renders):
    /// hands the engine back so it can be moved to the next stream.
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            let _ = self.give_back.send(engine);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_denormals_sets_flag_on_this_thread() {
        std::thread::spawn(|| {
            enable_flush_denormals();
            #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
            {
                assert!(flush_denormals_enabled());
                // A denormal product flushes to zero.
                let tiny = std::hint::black_box(f32::MIN_POSITIVE);
                let r = std::hint::black_box(tiny * std::hint::black_box(0.5f32));
                assert_eq!(r, 0.0);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn renders_in_chunks_and_hands_engine_back() {
        let parts = ether_core::create(ether_core::EngineConfig {
            max_block_size: 64,
            ..Default::default()
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(AudioShared::default());
        let mut r = RtRenderer::new(Box::new(parts.engine), shared.clone(), tx);
        let mut buf = vec![1.0f32; 200 * 4];
        r.render(&mut buf, 4);
        assert!(buf.iter().all(|s| *s == 0.0));
        let mut mono = vec![1i16; 100];
        r.render(&mut mono, 1);
        assert!(mono.iter().all(|s| *s == 0));
        assert_eq!(shared.frames.load(Ordering::Relaxed), 300);
        drop(r);
        assert!(rx.try_recv().is_ok());
    }

    /// `external-instrument`: an External Audio Effect's hardware send reaches device
    /// channels 3/4 (the master stays on 1/2), and the clock follows the engine.
    #[test]
    fn hardware_sends_reach_extra_device_channels() {
        use ether_core::graph::{ChainEntry, RenderGraphDesc};
        use ether_core::hw_io::HwIoDesc;
        use ether_core::protocol::model::{
            BuiltinDevice, BuiltinDeviceType, ExternalRouting, HwChannels, TrackId, TrackKind, Ulid,
        };

        struct Dc;
        impl ether_core::Node for Dc {
            fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
            fn reset(&mut self) {}
            fn process(
                &mut self,
                _: &mut ether_core::ProcessContext<'_>,
                audio: &mut ether_core::AudioBuffers<'_, '_>,
            ) -> ether_core::ProcessStatus {
                for ch in audio.outputs.iter_mut() {
                    ch.fill(0.5);
                }
                ether_core::ProcessStatus::Continue
            }
            fn channels(&self) -> (u16, u16) {
                (0, 2)
            }
        }

        let mut parts = ether_core::create(ether_core::EngineConfig {
            max_block_size: 64,
            max_nodes: 8,
            ..Default::default()
        });
        let dc = parts.handle.add_node(Box::new(Dc)).unwrap();
        let mut fx = ether_devices::create(
            &BuiltinDevice::new(BuiltinDeviceType::ExternalAudioEffect),
            &ether_devices::NoSamples,
        );
        fx.prepare(&ether_core::PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 64,
            max_events_per_block: 16,
        });
        fx.set_param(ether_devices::external::external_audio_effect::MIX, 0.0);
        let fx = parts.handle.add_node(fx).unwrap();
        let json = serde_json::json!({
            "version": 1, "tempo": [], "signatures": [], "loop_enabled": false,
            "loop_start": 0.0, "loop_end": 0.0, "metronome": false, "tracks": []
        });
        let mut desc: RenderGraphDesc = serde_json::from_value(json).unwrap();
        let track = |id: u128, kind, output: Option<TrackId>| {
            let mut t = serde_json::from_value::<ether_core::graph::TrackDesc>(serde_json::json!({
                "id": TrackId(Ulid(id)), "kind": kind, "chain": [], "output": output,
                "group": null, "sends": [], "volume": 1.0, "pan": 0.0, "mute": false,
                "solo": false, "audio_input": null, "monitor": false, "armed": false,
                "clips": [], "automation": [], "racks": []
            }))
            .unwrap();
            t.output = output;
            t
        };
        let mut t = track(2, TrackKind::Audio, Some(TrackId(Ulid(1))));
        t.chain = [dc, fx]
            .into_iter()
            .map(|node| ChainEntry {
                node,
                enabled: true,
                sidechain: None,
            })
            .collect();
        t.hw_io = vec![HwIoDesc {
            node: fx,
            routing: ExternalRouting {
                audio_send: Some(HwChannels { first: 2, count: 2 }),
                ..ExternalRouting::default()
            },
        }];
        desc.tracks = vec![track(1, TrackKind::Master, None), t];
        parts.handle.publish(desc).unwrap();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(AudioShared::default());
        let mut r = RtRenderer::new(Box::new(parts.engine), shared.clone(), tx);
        let mut buf = vec![0.0f32; 64 * 6];
        r.render(&mut buf, 6);
        r.render(&mut buf, 6);
        let frame = &buf[6 * 10..6 * 11];
        // Mix 0: the dry signal on the master; the send on 3/4; 5/6 silent.
        assert_eq!(frame, &[0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        assert_eq!(shared.output_channels.load(Ordering::Relaxed), 6);
        assert_eq!(shared.clock.read().1, 64);
        assert!(shared.clock.sample_now(48_000.0).unwrap() >= 64.0);
    }
}
