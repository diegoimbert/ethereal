//! `cargo run -p ether-protocol --example gen-ts -- <out_dir>` (see `just gen-types`).

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ui/src/generated".into());
    ether_protocol::ts::export_all(std::path::Path::new(&dir)).expect("TS export failed");
    println!("TypeScript types written to {dir}");
}
