// The crate's integration tests, one module per file, in one test binary: a test binary per file
// links the whole dependency graph each time (on Windows ~30 min of CI for ~200 of them).
// Add a file here and its `mod` line below (scripts/ci/test-layout.test.mjs checks). A test
// that needs a process of its own (its own global allocator, `harness = false`) stays a file
// of tests/.
#[path = "../fake/mod.rs"]
mod fake;
mod file_filters;
mod file_import;
mod folder_import;
mod ir_swap;
mod latency_republish;
mod media_stream;
mod pipeline;
mod project_load_quanta;
mod stuck_notes;
#[cfg(feature = "bench")]
mod web_perf;
