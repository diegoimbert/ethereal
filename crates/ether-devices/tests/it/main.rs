// The crate's integration tests, one module per file, in one test binary: a test binary per file
// links the whole dependency graph each time (on Windows ~30 min of CI for ~200 of them).
// Add a file here and its `mod` line below (scripts/ci/test-layout.test.mjs checks). A test
// that needs a process of its own (its own global allocator, `harness = false`) stays a file
// of tests/.
#[path = "../quality_rig/mod.rs"]
mod quality_rig;

// The no-allocation checks (assert_no_alloc) of every module: they only act inside
// `assert_no_alloc` scopes, per thread.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod compressor_detector;
mod device_ui_layouts;
mod devices2;
mod drum_rack;
mod effects_defaults;
mod effects_features;
mod engine_text;
mod eq_analysis;
mod external;
mod external_engine;
mod fm_synth;
mod fx_analysis;
mod fx_color;
mod fx_dynamics;
mod fx_modulation;
mod fx_space;
mod implemented;
mod instruments_golden;
mod instruments_poly_synth;
mod instruments_synth_sampler;
mod layouts;
mod midi_fx;
mod multisampler;
mod note_blocker;
mod perf_devices;
mod poly_synth;
mod racks;
mod render;
mod sidechain;
mod silence_skipping;
mod sound_quality;
mod v02_descriptors;
mod v03_descriptors;
mod vibrato;
mod volume_lfo;
