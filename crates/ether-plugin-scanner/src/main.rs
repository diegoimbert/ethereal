//! `ether-plugin-scanner`: scans CLAP bundles out-of-process.
//!
//! Modes:
//! - no arguments (the host protocol, see `ether_protocol::plugins`): reads a JSON
//!   `ScanRequest` from stdin, loads that ONE bundle, writes one JSON `ScanResponse` line to
//!   stdout, exits 0. A crash/hang only loses that bundle; the host
//!   (`ether_clap::ScanRunner`) enforces a timeout.
//! - `--paths`: print the platform CLAP search paths (+ `CLAP_PATH`) as a JSON array.
//! - `--scan-all [DIR_OR_BUNDLE...]`: find every bundle under the given paths (default: the
//!   platform search paths), scan each one in a child scanner process (this same binary, in
//!   protocol mode) and print `{"plugins": [PluginDescriptor...], "failed": [ScanFailure...]}`.
//!   Options: `--timeout-ms N` (per bundle).
//!
//! The scanner creates no temp files or IPC objects (pipes only); anything added later must
//! be named with `ether_core::plugin::ipc_name`. Owned by the `clap` node.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => protocol(),
        Some("--paths") => {
            let paths: Vec<String> = ether_clap::default_search_paths()
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            println!("{}", serde_json::to_string(&paths).expect("serialize"));
        }
        Some("--scan-all") => scan_all(&args[1..]),
        Some(other) => {
            eprintln!(
                "unknown argument {other:?}; usage: ether-plugin-scanner [--paths | --scan-all [--timeout-ms N] [PATH...]]"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn protocol() {
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
    // Leading newline: a plugin may have printed a partial line to stdout while loading.
    println!(
        "\n{}",
        serde_json::to_string(&response).expect("serialize response")
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn scan_all(args: &[String]) {
    use std::path::PathBuf;
    use std::time::Duration;

    let mut timeout = ether_clap::ScanRunner::DEFAULT_TIMEOUT;
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--timeout-ms" {
            let ms = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                eprintln!("--timeout-ms needs a number");
                std::process::exit(2)
            });
            timeout = Duration::from_millis(ms);
        } else {
            paths.push(PathBuf::from(a));
        }
    }
    if paths.is_empty() {
        paths = ether_clap::default_search_paths();
    }
    let exe = std::env::current_exe().expect("current exe");
    let runner = ether_clap::ScanRunner::new(exe).with_timeout(timeout);
    let bundles = ether_clap::find_bundles(&paths);
    let report = runner.scan_all(&bundles, |done, total, current| {
        if let Some(current) = current {
            eprintln!("[{}/{}] {}", done + 1, total, current.display());
        }
    });
    let json = serde_json::json!({ "plugins": report.plugins, "failed": report.failed });
    println!("{json}");
}

#[cfg(target_arch = "wasm32")]
fn main() {}
