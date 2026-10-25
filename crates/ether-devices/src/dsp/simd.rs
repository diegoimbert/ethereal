//! Runtime CPU dispatch for hot loops: the portable build targets baseline x86-64 (SSE2), so
//! a hot function can also be compiled for AVX2 + FMA and picked per call on CPUs that have
//! them.
//!
//! The pattern (one wrapper per hot entry point, called per block, not per sample):
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
