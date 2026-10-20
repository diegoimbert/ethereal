//! `ether-server` binary: see `ether-server --help` and the crate docs ([`ether_server::app`]).

fn main() {
    ether_server::app::main(ether_server::app::Hooks::default());
}
