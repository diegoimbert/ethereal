//! Collaboration through a real relay: two controllers, real WebSockets (tungstenite
//! client threads), `ether-collab-relay`'s server with a token.

#[path = "collab_support.rs"]
mod support;

use std::time::{Duration, Instant};

use ether_collab::relay::server::{RelayServer, RelayServerConfig};
use ether_controller::Controller;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::collab::CollabStatus;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use support::*;

const TOKEN: &str = "relay-test-token";

fn relay() -> RelayServer {
    RelayServer::start(RelayServerConfig {
        token: Some(TOKEN.into()),
        ..RelayServerConfig::default()
    })
    .expect("relay starts")
}

fn site(seed: u64) -> Site {
    Site::new(
        seed,
        ether_collab::default_connector(),
        MemoryLibrary::new(),
    )
}

/// Tick `sites` until `done` holds (real network: bounded wait).
fn wait(sites: &mut [&mut Site], what: &str, done: impl Fn(&[&mut Site]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        for s in sites.iter_mut() {
            s.tick();
        }
        if done(sites) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn two_controllers_through_a_real_relay() {
    let relay = relay();
    let url = relay.url();
    let mut a = site(101);
    let mut b = site(202);
    a.create_project("Relay");
    let t: TrackId = a.id();
    a.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Midi,
        name: Some("drums".into()),
        color: None,
        parent: None,
        before: None,
    }));
    a.join(&url, "live", "A", Some(TOKEN));
    wait(&mut [&mut a], "A online", |s| s[0].online());
    b.join(&url, "live", "B", Some(TOKEN));
    wait(&mut [&mut a, &mut b], "B joined", |s| {
        s[1].online()
            && s[1]
                .ctl
                .project()
                .is_some_and(|p| p.tracks.contains_key(&t))
    });
    assert_eq!(b.project().tracks[&t].name, "drums");
    assert_eq!(relay.with_relay(|r| r.peer_count("live")), 2);

    // Edits both ways.
    b.ok(Command::Mixer(MixerCommand::SetVolume {
        track: t,
        volume: Decibels(-7.5),
    }));
    wait(&mut [&mut a, &mut b], "B's edit on A", |s| {
        s[0].project().tracks[&t].mixer.volume == Decibels(-7.5)
    });
    let u: TrackId = a.id();
    a.ok(Command::Track(TrackCommand::Create {
        id: u,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    wait(&mut [&mut a, &mut b], "A's edit on B", |s| {
        s[1].project().tracks.contains_key(&u) && s[0].ctl.collab_pending() == 0
    });
    assert_eq!(shared(a.project()), shared(b.project()));
    wait(&mut [&mut a, &mut b], "presence", |s| {
        s[0].peers().iter().any(|p| p.name == "B") && s[1].peers().iter().any(|p| p.name == "A")
    });
}

#[test]
fn a_wrong_token_is_refused() {
    let relay = relay();
    let mut a = site(303);
    a.create_project("X");
    a.join(&relay.url(), "live", "A", Some("wrong"));
    wait(&mut [&mut a], "refusal", |s| {
        s[0].status() == Some(CollabStatus::Offline)
    });
    assert!(a.log.iter().any(|m| matches!(
        m,
        ServerMessage::Event(Event::Notification { message, .. }) if message.contains("invalid token")
    )));
    assert_eq!(relay.with_relay(|r| r.session_count()), 0);
}
