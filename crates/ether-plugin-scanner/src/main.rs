//! `ether-plugin-scanner`: scans plugins (CLAP, VST3, VST2, AU) out-of-process.
//!
//! Modes:
//! - no arguments (the host protocol, see `ether_protocol::plugins`): reads a JSON
//!   `ScanRequest` from stdin, loads that ONE target through the format's
//!   `PluginFormatHost` (format from the request, else inferred from the path: `.clap`,
//!   `.vst3`, `.dll`/`.so`/`.vst` (VST2), AU component id), writes one JSON `ScanResponse` line to stdout, exits 0. A
//!   crash/hang only loses that target; the host (`ether_plugin_host::ScanRunner`) enforces a
//!   timeout.
//! - `--paths`: print every format's platform search paths (CLAP first, incl. `CLAP_PATH`) as
//!   a JSON array.
//! - `--scan-all [DIR_OR_BUNDLE...]`: find every scan target under the given paths (default:
//!   every format's search paths plus the AU component registry), scan each one in a child
//!   scanner process (this same binary, in protocol mode) and print
//!   `{"plugins": [PluginDescriptor...], "failed": [ScanFailure...]}`.
//!   Options: `--timeout-ms N` (per target).
//!
//! The scanner creates no temp files or IPC objects (pipes only); anything added later must
//! be named with `ether_core::plugin::ipc_name`.

#[cfg(not(target_arch = "wasm32"))]
fn formats() -> ether_plugin_host::Formats {
    use std::sync::Arc;
    ether_plugin_host::Formats::new(vec![
        Arc::new(ether_clap::ClapFormat),
        Arc::new(ether_vst3::Vst3Format),
        Arc::new(ether_vst2::Vst2Format),
        Arc::new(ether_au::AuFormat),
    ])
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => protocol(),
        Some("--paths") => {
            let paths: Vec<String> = formats()
                .default_search_paths()
                .iter()
                .map(|(_, p)| p.to_string_lossy().into_owned())
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
            Ok(req) => formats().handle_scan_request(&req),
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

    let mut timeout = ether_plugin_host::ScanRunner::DEFAULT_TIMEOUT;
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
    let exe = std::env::current_exe().expect("current exe");
    let runner = ether_plugin_host::ScanRunner::new(exe).with_timeout(timeout);
    let targets = formats().discover((!paths.is_empty()).then_some(paths.as_slice()));
    let report = runner.scan_targets(&targets, |done, total, current| {
        if let Some(current) = current {
            eprintln!("[{}/{}] {}", done + 1, total, current.display());
        }
    });
    let json = serde_json::json!({ "plugins": report.plugins, "failed": report.failed });
    println!("{json}");
}

#[cfg(target_arch = "wasm32")]
fn main() {}
