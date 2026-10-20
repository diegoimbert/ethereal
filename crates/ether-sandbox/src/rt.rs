//! Real-time setup of the helper's audio thread: flush-to-zero
//! and real-time scheduling, like the engine's own audio and worker threads.
//!
//! Mirrors `ether_native::rt::enable_flush_denormals` and the worker promotion of
//! `ether_native::workers` (`ether-native` depends on this crate, not the reverse, and the
//! helper binary must stay free of the native host).

use std::time::Duration;

/// Enable flush-to-zero / denormals-are-zero on the **current thread** (x86: MXCSR FTZ
/// and DAZ; aarch64: FPCR FZ; else no-op). Denormals in a plugin's decaying tails can be
/// ~100× slower and blow the block deadline.
pub fn enable_flush_denormals() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: reads/writes only this thread's MXCSR control bits; FTZ/DAZ never trap.
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

/// Whether flush-to-zero is active on the current thread (tests).
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

/// Raise the current thread to real-time scheduling for blocks of `period` (Linux:
/// `SCHED_FIFO` 60, below typical audio-thread priorities; macOS: time-constraint policy).
/// Silently stays normal without the rights to do so.
pub fn promote_current_thread(period: Duration) {
    os::promote(period);
}

#[cfg(target_os = "linux")]
mod os {
    use std::time::Duration;

    #[repr(C)]
    struct SchedParam {
        sched_priority: i32,
    }

    const SCHED_FIFO: i32 = 1;

    unsafe extern "C" {
        fn pthread_self() -> std::ffi::c_ulong;
        fn pthread_setschedparam(
            thread: std::ffi::c_ulong,
            policy: i32,
            param: *const SchedParam,
        ) -> i32;
    }

    pub(super) fn promote(_period: Duration) {
        let param = SchedParam { sched_priority: 60 };
        // SAFETY: valid thread handle (this thread) and param struct.
        let r = unsafe { pthread_setschedparam(pthread_self(), SCHED_FIFO, &param) };
        if r != 0 {
            tracing::debug!(r, "sandbox helper: SCHED_FIFO refused");
        }
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::time::Duration;

    #[repr(C)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }

    #[repr(C)]
    struct TimeConstraintPolicy {
        period: u32,
        computation: u32,
        constraint: u32,
        preemptible: u32,
    }

    const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
    const THREAD_TIME_CONSTRAINT_POLICY_COUNT: u32 = 4;

    unsafe extern "C" {
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
        fn pthread_self() -> usize;
        fn pthread_mach_thread_np(thread: usize) -> u32;
        fn thread_policy_set(thread: u32, flavor: u32, info: *mut u32, count: u32) -> i32;
    }

    pub(super) fn promote(period: Duration) {
        let mut tb = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: plain out-parameter call.
        if unsafe { mach_timebase_info(&mut tb) } != 0 || tb.numer == 0 {
            return;
        }
        let to_abs = |d: Duration| {
            (d.as_nanos() as u64 * tb.denom as u64 / tb.numer as u64).min(u32::MAX as u64) as u32
        };
        let period = period.max(Duration::from_micros(500));
        let mut policy = TimeConstraintPolicy {
            period: to_abs(period),
            computation: to_abs(period / 2),
            constraint: to_abs(period),
            preemptible: 1,
        };
        // SAFETY: `policy` is the documented 4 × integer_t layout of
        // `thread_time_constraint_policy_data_t`; the thread port is this thread's.
        let r = unsafe {
            thread_policy_set(
                pthread_mach_thread_np(pthread_self()),
                THREAD_TIME_CONSTRAINT_POLICY,
                (&mut policy as *mut TimeConstraintPolicy).cast(),
                THREAD_TIME_CONSTRAINT_POLICY_COUNT,
            )
        };
        if r != 0 {
            tracing::debug!(r, "sandbox helper: time-constraint policy refused");
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod os {
    pub(super) fn promote(_period: std::time::Duration) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_denormals_on_this_thread_only() {
        std::thread::spawn(|| {
            enable_flush_denormals();
            #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
            assert!(flush_denormals_enabled());
            // Promotion without the rights is a silent no-op.
            promote_current_thread(Duration::from_millis(5));
        })
        .join()
        .unwrap();
    }
}
