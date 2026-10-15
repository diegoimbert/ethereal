//! The 1 ms Windows timer while the engine runs.
//!
//! Windows' default timer period is 15.6 ms, so a `Sleep(1)` sleeps ~15.6 ms. Plugins that
//! wait in `Sleep(1)` loops while they load (Waves: their open, state restore and activate
//! each wait like this) took ~3 s per step: ~9.7 s per J37 instance, ~1.4 s with a 1 ms
//! period (measured). DAWs raise the period for their lifetime;
//! so does the engine host, and restores it when stopped. No-op elsewhere.

/// Holds the 1 ms timer period until dropped.
pub(crate) struct HighResolutionTimer {
    #[cfg(windows)]
    active: bool,
}

impl HighResolutionTimer {
    pub(crate) fn acquire() -> Self {
        #[cfg(windows)]
        {
            // SAFETY: plain call; balanced by `timeEndPeriod` in `drop` when it succeeded.
            let active = unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) } == 0;
            if !active {
                tracing::warn!("could not raise the Windows timer resolution to 1 ms");
            }
            Self { active }
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }
}

impl Drop for HighResolutionTimer {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.active {
            // SAFETY: balances the successful `timeBeginPeriod(1)` above.
            unsafe { windows_sys::Win32::Media::timeEndPeriod(1) };
        }
    }
}
