// The crate's integration tests, one module per file, in one test binary: a test binary per file
// links the whole dependency graph each time (on Windows ~30 min of CI for ~200 of them).
// Add a file here and its `mod` line below (scripts/ci/test-layout.test.mjs checks). A test
// that needs a process of its own (its own global allocator, `harness = false`) stays a file
// of tests/.
mod format_fixtures;
mod index;
mod model_rules;
mod nested_sidechain;
mod ops_proptest;
mod plugin_formats;
mod roadmap_v2;
mod roadmap_v3;
mod roadmap_v4;
mod social;
mod tempo_vectors;
