//! Which patch shapes the controller's receivers understand (perf-plugins, audit #4).
//!
//! `PatchChange::DeviceParam` is additive: UIs built with it apply it, older ones would
//! drop it silently and drift. Embedded UIs (desktop, web) always ship with their engine,
//! so the default is field-level patches. A remote engine (`ether-server`) registers every
//! connected client that did not announce `remote::FEATURE_DEVICE_PARAM_PATCH` in its
//! hello; while one is connected, the controller sends whole-device upserts instead
//! (`ether_model::upsert_device_params`), so old clients keep working.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shared switch between a host's connections and its controller (cheap to clone).
#[derive(Clone, Debug, Default)]
pub struct PatchCompat(Arc<AtomicUsize>);

impl PartialEq for PatchCompat {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PatchCompat {
    /// `true` while no connected receiver predates `PatchChange::DeviceParam`.
    pub fn field_patches(&self) -> bool {
        self.0.load(Ordering::Acquire) == 0
    }

    /// Register a receiver without `DeviceParam` support until the guard is dropped.
    pub fn legacy_client(&self) -> LegacyClient {
        self.0.fetch_add(1, Ordering::AcqRel);
        LegacyClient(self.0.clone())
    }
}

/// A connected receiver that needs whole-device upserts (see [`PatchCompat`]).
#[derive(Debug)]
pub struct LegacyClient(Arc<AtomicUsize>);

impl Drop for LegacyClient {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_clients_switch_field_patches_off_while_connected() {
        let compat = PatchCompat::default();
        assert!(compat.field_patches());
        let a = compat.legacy_client();
        let b = compat.clone().legacy_client();
        assert!(!compat.field_patches());
        drop(a);
        assert!(!compat.field_patches());
        drop(b);
        assert!(compat.field_patches());
        assert_eq!(compat, compat.clone());
        assert_ne!(compat, PatchCompat::default());
    }
}
