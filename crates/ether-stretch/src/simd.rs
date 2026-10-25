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
//!
//! (The same helper as `ether-devices`' `dsp::simd`; this crate doesn't depend on it.)

/// Whether this CPU runs the AVX2 + FMA variants (std caches the detection: one load).
#[inline]
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
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
            if $crate::simd::avx2() {
                // SAFETY: `avx2()` checked that the CPU has AVX2 and FMA.
                return unsafe { self.$fast($($arg),*) };
            }
            self.$generic($($arg),*)
        }

        $(#[$meta])*
        #[cfg(target_arch = "x86_64")]
        #[target_feature(enable = "avx2,fma")]
        fn $fast(&mut self $(, $arg: $ty)*) $(-> $ret)? {
            self.$generic($($arg),*)
        }
    };
}
pub(crate) use avx2_dispatch;
