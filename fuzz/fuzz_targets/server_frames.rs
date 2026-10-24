//! Client frames of the engine server (text JSON and binary), after the handshake.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ether_server::frames::decode_client_binary(data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = ether_server::frames::decode_client_text(text);
    }
});
