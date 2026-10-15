//! Builds the vendored Signalsmith Stretch (`csrc/`) when the `signalsmith` feature is on
//! and the target is native (the C++ isn't built for wasm32).

fn main() {
    println!("cargo::rerun-if-changed=csrc");
    let feature = std::env::var_os("CARGO_FEATURE_SIGNALSMITH").is_some();
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    if !feature || wasm {
        return;
    }
    cc::Build::new()
        .cpp(true)
        .std("c++14")
        .include("csrc")
        .file("csrc/wrapper.cpp")
        // Signalsmith's own code is not ours to lint.
        .warnings(false)
        .compile("ether_signalsmith");
}
