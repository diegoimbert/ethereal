//! Runtime CPU dispatch for hot loops: the portable build targets baseline x86-64 (SSE2), so
//! a hot function can also be compiled for AVX2 + FMA and picked per call on CPUs that have
//! them.
//!
//! The pattern (one wrapper per hot entry point, called per block, not per sample;
//! [`avx2_dispatch!`] writes the first two methods):
//!
//! ```ignore
//! pub fn process(&mut self, ..) {
//!     #[cfg(target_arch = "x86_64")]
//!     if simd::avx2() {
//!         // SAFETY: `avx2()` checked that the CPU has AVX2 and FMA.
//!         return unsafe { self.process_avx2(..) };
//!     }
//!     self.process_generic(..)
//! }
//!
//! #[cfg(target_arch = "x86_64")]
//! #[target_feature(enable = "avx2,fma")]
//! fn process_avx2(&mut self, ..) { self.process_generic(..) }
//!
//! #[inline(always)]
//! fn process_generic(&mut self, ..) { .. }
//! ```
//!
//! Output is bit-identical to the baseline build: Rust never contracts `a * b + c` into an
//! FMA (only an explicit `mul_add` is fused, and that is correctly rounded either way) and
//! doesn't reassociate floats, so wider vectors compute the same operations in the same
//! order. Unit tests compare both variants bit for bit ([`force_generic`]).

/// Whether this CPU runs the AVX2 + FMA variants (std caches the detection: one load).
#[inline]
#[cfg_attr(not(any(test, target_arch = "x86_64")), allow(dead_code))]
pub(crate) fn avx2() -> bool {
    #[cfg(test)]
    if FORCE_GENERIC.with(std::cell::Cell::get) {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

#[cfg(test)]
thread_local! {
    static FORCE_GENERIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only: run `f` with the baseline (non-AVX2) variants on this thread.
#[cfg(test)]
pub(crate) fn force_generic<R>(f: impl FnOnce() -> R) -> R {
    FORCE_GENERIC.with(|c| c.set(true));
    let r = f();
    FORCE_GENERIC.with(|c| c.set(false));
    r
}

/// Writes the dispatching method `$name` and its AVX2 + FMA variant `$fast`, both calling the
/// `#[inline(always)]` method `$generic` (written by hand, with the body), so the body is
/// compiled twice (module docs):
///
/// ```ignore
/// impl Dev {
///     avx2_dispatch! {
///         /// RT. One block.
///         pub fn render => render_avx2, render_generic(&mut self, out: &mut [f32]) -> bool
///     }
///
///     #[inline(always)]
///     fn render_generic(&mut self, out: &mut [f32]) -> bool { .. }
/// }
/// ```
macro_rules! avx2_dispatch {
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident => $fast:ident, $generic:ident
            (&mut self $(, $arg:ident: $ty:ty)* $(,)?) $(-> $ret:ty)?
    ) => {
        $(#[$meta])*
        $vis fn $name(&mut self $(, $arg: $ty)*) $(-> $ret)? {
            #[cfg(target_arch = "x86_64")]
            if $crate::dsp::simd::avx2() {
                // SAFETY: `avx2()` checked that the CPU has AVX2 and FMA.
                return unsafe { self.$fast($($arg),*) };
            }
            self.$generic($($arg),*)
        }

        #[cfg(target_arch = "x86_64")]
        #[target_feature(enable = "avx2,fma")]
        fn $fast(&mut self $(, $arg: $ty)*) $(-> $ret)? {
            self.$generic($($arg),*)
        }
    };
}
pub(crate) use avx2_dispatch;

#[cfg(test)]
mod tests {
    use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
    use ether_core::{
        AudioBuffers, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
        TransportInfo,
    };

    use super::*;
    use crate::NoSamples;

    const SR: f32 = 48_000.0;
    const BLOCK: usize = 128;

    /// Half a second of every device at defaults (noise in, notes for instruments), both
    /// channels, in blocks.
    fn render(ty: BuiltinDeviceType) -> Vec<f32> {
        let mut d = crate::create(&BuiltinDevice::new(ty), &NoSamples);
        d.prepare(&PrepareConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_events_per_block: 64,
        });
        let frames = 24_064;
        let mut seed = 5u32;
        let noise: Vec<f32> = (0..frames)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
            })
            .collect();
        let (mut ol, mut or) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        let mut out_events = EventBuffer::with_capacity(256);
        let sidechain = d.sidechain_inputs() > 0;
        let mut out = Vec::with_capacity(2 * frames);
        let mut pos = 0;
        while pos + BLOCK <= frames {
            let events = match pos {
                0 => vec![ProcessEvent {
                    offset: 3,
                    kind: EventKind::NoteOn {
                        note_id: 1,
                        channel: 0,
                        key: 60,
                        velocity: 0.8,
                    },
                }],
                12_032 => vec![ProcessEvent {
                    offset: 5,
                    kind: EventKind::NoteOff {
                        note_id: 1,
                        channel: 0,
                        key: 60,
                        velocity: 0.5,
                    },
                }],
                _ => Vec::new(),
            };
            let transport = TransportInfo {
                playing: true,
                sample_time: pos as u64,
                bpm: 120.0,
                beats_per_sample: 2.0 / SR as f64,
                position: pos as f64 * 2.0 / SR as f64,
                ..TransportInfo::STOPPED
            };
            let input = &noise[pos..pos + BLOCK];
            let inputs: [&[f32]; 2] = [input, input];
            let mut outputs: [&mut [f32]; 2] = [&mut ol, &mut or];
            let mut ctx = ProcessContext {
                sample_rate: SR,
                frames: BLOCK,
                transport: &transport,
                events: &events,
                out_events: &mut out_events,
            };
            let mut audio = AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            };
            if sidechain {
                d.process_sidechain(&mut ctx, &mut audio, &[input, input]);
            } else {
                d.process(&mut ctx, &mut audio);
            }
            out.extend_from_slice(&ol);
            out.extend_from_slice(&or);
            out_events.clear();
            pos += BLOCK;
        }
        out
    }

    /// Every built-in renders the same bits with the AVX2 variants as without (a no-op on
    /// CPUs without AVX2).
    #[test]
    fn avx2_variants_are_bit_identical() {
        if !avx2() {
            return;
        }
        for ty in BuiltinDeviceType::ALL {
            let fast = render(ty);
            let generic = force_generic(|| render(ty));
            let same = fast
                .iter()
                .zip(&generic)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same, "{ty:?}: AVX2 output differs");
        }
    }
}
