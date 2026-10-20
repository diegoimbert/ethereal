//! Test helpers (no `tempfile` dependency in the workspace).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A unique temp directory, removed on drop.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ether-native-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A bridge over a fresh engine (not running) whose plugins are `instantiate`'s, with one
/// catalog entry: the fake CLAP effect `fake` (feature `test-support`; [`crate::plugins::fake`]).
#[cfg(any(test, feature = "test-support"))]
pub fn fake_bridge(
    instantiate: crate::plugins::Instantiate,
) -> (crate::bridge::NativeBridge, ether_core::Engine) {
    use std::sync::Arc;

    use ether_core::protocol::model::PluginFormat;
    use ether_core::protocol::plugins::PluginDescriptor;

    let ether_core::EngineParts { engine, handle, .. } =
        ether_core::create(ether_core::EngineConfig {
            max_block_size: 128,
            ..Default::default()
        });
    let catalog = crate::plugins::PluginCatalog::default();
    catalog.replace(vec![PluginDescriptor {
        sidechain_inputs: Default::default(),
        format: PluginFormat::Clap,
        id: "fake".into(),
        name: "Fake".into(),
        vendor: "T".into(),
        version: "1".into(),
        description: String::new(),
        features: vec![],
        category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
        path: "/fake.clap".into(),
    }]);
    let b = crate::bridge::NativeBridge::new(
        handle,
        ether_core::PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 128,
            max_events_per_block: 64,
        },
        crate::plugins::PluginHost::new(Arc::new(crate::plugins::DedicatedThread::new())),
        catalog,
        instantiate,
        Arc::new(crate::rt::AudioShared::default()),
    );
    (b, engine)
}
