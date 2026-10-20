//! `ether-plugin-scanner`: the out-of-process plugin scanner (see the library's docs).

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    ether_plugin_scanner::main();
}
