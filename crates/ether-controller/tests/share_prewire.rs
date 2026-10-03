//! base-115 pre-wiring (docs/SHARING.md): what works before the sharing nodes land
//! (`Share::Get` reports `Off`), and what replies `Unsupported` until then. Each node removes
//! ONLY its own `Unsupported` assertions here (shared file) and tests its behaviour in its
//! own test file.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::collab::StreamSignal;
use ether_core::protocol::share::*;
use ether_core::protocol::*;
use support::*;

fn error_code(out: &[ServerMessage]) -> ErrorCode {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.code,
        other => panic!("expected an error reply, got {other:#?}"),
    }
}

#[test]
fn get_reports_off() {
    let hub = Hub::default();
    let mut s = Site::on_hub(1, &hub);
    let out = s.send(Command::Share(ShareCommand::Get));
    let states: Vec<_> = out
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Share {
                event: ShareEvent::State { state },
            }) => Some(state.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(states, [ShareState::Off]);
}

#[test]
fn everything_else_is_unsupported_until_its_node_lands() {
    let hub = Hub::default();
    let mut s = Site::on_hub(1, &hub);
    s.create_project("Song");
    // share-engine: implemented (tests/share.rs).
    for c in [
        // p2p-transport (web UI endpoint)
        ShareCommand::PeerSignal {
            peer: 1,
            signal: StreamSignal::Bye { reason: None },
        },
    ] {
        let out = s.send(Command::Share(c.clone()));
        assert_eq!(error_code(&out), ErrorCode::Unsupported, "{c:?}");
    }
}
