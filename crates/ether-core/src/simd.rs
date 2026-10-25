//! Runtime CPU dispatch for hot loops: the portable build targets baseline x86-64 (SSE2), so
//! a hot function can also be compiled for AVX2 + FMA and picked per call on CPUs that have
//! them.
//!
//! [`avx2_fn!`] stamps one function body twice: once plain, once in a
//! `#[target_feature(enable = "avx2,fma")]` function, with a wrapper picking per call (once
//! per block or clip, not per sample). The body is stamped (not inlined) so its closures get
//! the features too.
//!
//! Output is bit-identical to the baseline build: Rust never contracts `a * b + c` into an
//! FMA (only an explicit `mul_add` is fused, and that is correctly rounded either way) and
//! doesn't reassociate floats, so wider vectors compute the same operations in the same
//! order. Unit tests compare both variants bit for bit ([`force_generic`]).
//!
//! (`ether-devices`' `dsp::simd` and `ether-stretch`'s `simd` are the same helper for
//! methods; neither crate is a dependency here.)

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

/// A free function compiled twice from one body: `$name` dispatches between `$fast` (AVX2 +
/// FMA) and `$generic`. Unlike [`avx2_dispatch!`] the body itself is stamped into the
/// `#[target_feature]` function, so closures in it (which are separate functions, never
/// covered by inlining a generic body) get the features too.
macro_rules! avx2_fn {
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident => $fast:ident, $generic:ident
            ($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)? $body:block
    ) => {
        $(#[$meta])*
        $vis fn $name($($arg: $ty),*) $(-> $ret)? {
            #[cfg(target_arch = "x86_64")]
            if $crate::simd::avx2() {
                // SAFETY: `avx2()` checked that the CPU has AVX2 and FMA.
                return unsafe { $fast($($arg),*) };
            }
            $generic($($arg),*)
        }

        $(#[$meta])*
        #[cfg(target_arch = "x86_64")]
        #[target_feature(enable = "avx2,fma")]
        fn $fast($($arg: $ty),*) $(-> $ret)? $body

        $(#[$meta])*
        fn $generic($($arg: $ty),*) $(-> $ret)? $body
    };
}
pub(crate) use avx2_fn;
