// The crate's integration tests, one module per file, in one test binary: a test binary per file
// links the whole dependency graph each time (on Windows ~30 min of CI for ~200 of them).
// Add a file here and its `mod` line below (scripts/ci/test-layout.test.mjs checks). A test
// that needs a process of its own (its own global allocator, `harness = false`) stays a file
// of tests/.
#[path = "../common/mod.rs"]
mod common;

// The no-allocation checks (assert_no_alloc) of every module: they only act inside
// `assert_no_alloc` scopes, per thread.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod base24_hooks;
mod codec;
mod codec_fixture;
mod crossfades_render;
mod device_switch;
mod device_switch_modulation;
mod drum_rack;
mod drum_rack_no_alloc;
mod engine;
mod expression;
mod expression_no_alloc;
mod fades_render;
mod fades_vectors;
mod groups;
mod hw_io;
mod job_costs;
mod metronome;
mod metronome_no_alloc;
mod midi_fx_bypass;
mod midi_fx_engine;
mod modulation;
mod modulation_no_alloc;
mod mpe;
mod nested_sidechain;
mod no_alloc;
mod parallel;
mod parallel_graphs;
mod perf_engine;
mod preview_fade;
mod racks;
mod recording_no_alloc;
mod roadmap_v3_hooks;
mod roadmap_v4_hooks;
mod sample_accurate;
mod sample_accurate_no_alloc;
mod silence;
mod stream_tap;
mod stuck_notes;
mod suspend;
mod tap_recording;
mod tempo_vectors;
mod touching_notes;
