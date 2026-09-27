//! `ether-server` binary (stub; see the crate docs).

fn main() {
    if let Err(e) = ether_server::serve(ether_server::ServerConfig::default()) {
        eprintln!("ether-server: {e}");
        std::process::exit(1);
    }
}
