// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Debug builds: allocations inside `assert_no_alloc` regions (the audio callback) are
// reported (ether-native enables the `warn_debug` feature, so they log instead of abort).
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

fn main() {
    ether_desktop_lib::run();
}
