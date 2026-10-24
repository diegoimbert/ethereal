//! Standard MIDI files.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ether_media::smf::read(data, "fuzz.mid");
});
