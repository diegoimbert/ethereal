//! `.ether` project files: parse, migrate and validate.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(json) = std::str::from_utf8(data) {
        let _ = ether_model::file::load(json);
    }
});
