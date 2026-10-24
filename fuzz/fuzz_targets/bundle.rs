//! `.ether` bundles (the project and its media in one file).
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ether_controller::bundle::unpack(data);
});
