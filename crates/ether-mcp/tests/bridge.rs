//! The desktop agent bridge (`ether_server::agent_bridge`): runtime file (0600, removed on
//! disable), token auth, listener lifecycle.

mod support;

use std::net::TcpStream;

use ether_mcp::backend::Backend;
use ether_mcp::remote::{RemoteBackend, Target};
use ether_native::test_util::TempDir;
use ether_protocol::agent::AgentCommand;
use ether_protocol::{Command, ReplyValue};
use ether_server::agent_bridge::{RuntimeInfo, read_runtime_file};
use support::Desktop;

#[test]
fn runtime_file_token_and_disable() {
    let tmp = TempDir::new("bridge");
    let desktop = Desktop::start(tmp.path());
    let runtime = tmp.path().join("app").join("agent-bridge.json");
    let port = desktop.enable_bridge(runtime.clone());

    let info: RuntimeInfo = read_runtime_file(&runtime).expect("runtime file");
    assert_eq!(info.port, port);
    assert_eq!(info.pid, std::process::id());
    assert_eq!(info.token.len(), 48, "24 random bytes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&runtime).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only runtime file");
    }

    // A wrong (or missing) token is rejected.
    for token in [Some("0".repeat(48)), None] {
        let b = RemoteBackend::new(Target::Fixed {
            url: info.url(),
            token,
        });
        let e = b.connect().unwrap_err();
        assert!(e.contains("BadToken"), "{e}");
    }
    assert_eq!(desktop.bridge_clients(), 0);

    // The right one (read from the file, like ether-mcp's default mode) works.
    let b = RemoteBackend::new(Target::Desktop {
        candidates: vec![tmp.path().join("missing.json"), runtime.clone()],
    });
    let ReplyValue::AgentTools { tools } =
        b.request(Command::Agent(AgentCommand::ListTools)).unwrap()
    else {
        panic!("tools");
    };
    assert!(tools.iter().any(|t| t.name == "add_notes"));
    assert_eq!(desktop.bridge_clients(), 1);

    // Disable: listener closed, file gone, clients see a clear error.
    desktop.disable_bridge();
    assert!(!runtime.exists(), "runtime file removed on disable");
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "listener closed"
    );
    let e = b
        .request(Command::Agent(AgentCommand::ListTools))
        .unwrap_err();
    assert!(e.contains("Allow AI agents (MCP)"), "{e}");
}
