// The crate's integration tests, one module per file, in one test binary: a test binary per file
// links the whole dependency graph each time (on Windows ~30 min of CI for ~200 of them).
// Add a file here and its `mod` line below (scripts/ci/test-layout.test.mjs checks). A test
// that needs a process of its own (its own global allocator, `harness = false`) stays a file
// of tests/. A module that needs features is cfg'd on them here (cargo's `required-features`
// applies to whole binaries).
#[path = "../common/mod.rs"]
mod common;

// The no-allocation checks (assert_no_alloc) of every module: they only act inside
// `assert_no_alloc` scopes, per thread.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[cfg(feature = "plugins-clap")]
mod agent_plugins_e2e;
mod browser_e2e;
#[cfg(feature = "warp-complex")]
mod disk_stream_render;
mod drum_rack_e2e;
mod e2e_flow;
mod e2e_warp;
mod engine_bench;
mod export_e2e;
mod file_filters_e2e;
mod file_import_e2e;
#[cfg(all(
    feature = "plugins-clap",
    feature = "plugins-vst3",
    feature = "sandbox"
))]
mod formats_e2e;
mod media_preview_e2e;
mod media_refs_e2e;
mod multicore;
mod multicore_bench;
mod multicore_real;
mod null_host;
mod perf_audio;
#[cfg(feature = "plugins-clap")]
mod plugin_folders_e2e;
#[cfg(all(
    feature = "plugins-clap",
    feature = "plugins-vst3",
    feature = "sandbox"
))]
mod plugin_sidechain;
#[cfg(all(feature = "plugins-clap", feature = "sandbox"))]
mod plugins_e2e;
mod quit_save;
mod recording_e2e;
mod sidechain_render;
mod tempo_metronome;
