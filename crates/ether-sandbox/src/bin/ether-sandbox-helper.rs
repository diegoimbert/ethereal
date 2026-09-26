//! Helper process hosting one sandboxed plugin (see `ether_sandbox::helper`).
//! Usage: `ether-sandbox-helper <bundle.clap> <plugin-id>`; control over stdin/stdout, audio
//! over shared memory named by the host with `ether_core::plugin::ipc_name`.

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    std::process::exit(ether_sandbox::helper::main());
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!("ether-sandbox-helper: not supported on this platform yet");
    std::process::exit(2);
}
