//! `ether-plugin-scanner`: scans ONE CLAP bundle out-of-process.
//!
//! Protocol (see `ether_protocol::plugins`): reads a JSON `ScanRequest` from stdin, writes
//! one JSON `ScanResponse` to stdout, exits 0. A crash/hang only loses that bundle; the
//! host enforces a timeout. Any temp files/IPC names use `ether_core::plugin::ipc_name`.
//! Owned by the `clap` node.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use std::io::Read;

    use ether_core::protocol::plugins::{ScanRequest, ScanResponse};

    let mut input = String::new();
    let response = match std::io::stdin().read_to_string(&mut input) {
        Err(e) => ScanResponse::Err {
            message: format!("stdin: {e}"),
        },
        Ok(_) => match serde_json::from_str::<ScanRequest>(&input) {
            Err(e) => ScanResponse::Err {
                message: format!("bad request: {e}"),
            },
            Ok(req) => match ether_clap::scan_bundle(std::path::Path::new(&req.bundle_path)) {
                Ok(plugins) => ScanResponse::Ok { plugins },
                Err(e) => ScanResponse::Err {
                    message: e.to_string(),
                },
            },
        },
    };
    println!(
        "{}",
        serde_json::to_string(&response).expect("serialize response")
    );
}

#[cfg(target_arch = "wasm32")]
fn main() {}
