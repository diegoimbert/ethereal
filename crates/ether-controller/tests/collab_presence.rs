//! Presence v2 live pointers (docs/COLLAB.md §8.1, §8.3): `SetPointer` throttling (latest
//! wins, flushed on a later tick, clears never throttled), peers' pointers as
//! `CollabEvent::Pointer`, and a clear when a peer leaves.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::collab::*;
use ether_core::protocol::model::*;
use ether_core::protocol::*;
use support::*;

fn at(beats: f64) -> ArrangerPointer {
    ArrangerPointer {
        beats: Beats(beats),
        track: None,
        y: 0.0,
        editor: None,
    }
}

fn set(s: &mut Site, p: Option<ArrangerPointer>) {
    s.ok(Command::Collab(CollabCommand::SetPointer { pointer: p }));
}

/// `(site, pointer)` of every `CollabEvent::Pointer` in `out`.
fn pointers(out: &[ServerMessage]) -> Vec<(SiteId, Option<ArrangerPointer>)> {
    out.iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Collab {
                event: CollabEvent::Pointer { site, pointer },
            }) => Some((*site, pointer.clone())),
            _ => None,
        })
        .collect()
}

/// Deliver what the relay holds (relay clock = `now`) and tick the receiver once.
fn pump(hub: &Hub, now: u64, rx: &mut Site) -> Vec<(SiteId, Option<ArrangerPointer>)> {
    hub.tick(now);
    hub.deliver();
    hub.deliver();
    let now_rx = rx.ctl.host.now;
    rx.ctl.host.now = now.max(now_rx).saturating_sub(20);
    pointers(&rx.tick())
}

#[test]
fn pointer_reaches_peers_and_own_is_not_echoed() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let a = sites[0].ctl.collab_site();
    let now = sites[0].ctl.host.now;
    set(&mut sites[0], Some(at(4.0)));
    let got = pump(&hub, now, &mut sites[1]);
    assert_eq!(got, [(a, Some(at(4.0)))]);
    let own = pointers(&sites[0].tick());
    assert!(own.is_empty(), "own pointer dropped: {own:?}");
}

#[test]
fn positions_are_throttled_latest_wins_and_flushed_on_tick() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let a = sites[0].ctl.collab_site();
    let t0 = sites[0].ctl.host.now;
    // A burst of 10 moves within 10 ms: the first goes out, the rest collapse to the last.
    for i in 0..10 {
        sites[0].ctl.host.now = t0 + i;
        set(&mut sites[0], Some(at(i as f64)));
    }
    let got = pump(&hub, t0 + 10, &mut sites[1]);
    assert_eq!(
        got,
        [(a, Some(at(0.0)))],
        "only the first within the interval"
    );
    // A tick before the interval does not send.
    sites[0].ctl.host.now = t0 + 10;
    sites[0].tick(); // now t0 + 30
    assert!(pump(&hub, t0 + 30, &mut sites[1]).is_empty());
    // The next tick past the interval sends the latest value.
    sites[0].tick(); // now t0 + 50
    let got = pump(&hub, t0 + 50, &mut sites[1]);
    assert_eq!(got, [(a, Some(at(9.0)))], "latest wins");
}

#[test]
fn at_most_pointer_max_hz_while_moving() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t0 = sites[0].ctl.host.now;
    let mut received = 0;
    // One second of moves every 5 ms, ticking every 20 ms.
    for ms in (0..1000).step_by(5) {
        sites[0].ctl.host.now = t0 + ms;
        set(&mut sites[0], Some(at(ms as f64 / 100.0)));
        if ms % 20 == 0 {
            sites[0].ctl.host.now -= 20;
            sites[0].tick();
            received += pump(&hub, t0 + ms, &mut sites[1]).len();
        }
    }
    received += pump(&hub, t0 + 1000, &mut sites[1]).len();
    assert!(
        (20..=POINTER_MAX_HZ as usize).contains(&received),
        "{received} pointer messages in one second"
    );
}

#[test]
fn clears_are_never_throttled_and_drop_a_pending_position() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let a = sites[0].ctl.collab_site();
    let t0 = sites[0].ctl.host.now;
    set(&mut sites[0], Some(at(1.0)));
    sites[0].ctl.host.now = t0 + 5;
    set(&mut sites[0], Some(at(2.0))); // throttled
    sites[0].ctl.host.now = t0 + 6;
    set(&mut sites[0], None); // immediate, drops at(2.0)
    let got = pump(&hub, t0 + 6, &mut sites[1]);
    assert_eq!(got, [(a, Some(at(1.0))), (a, None)]);
    // Nothing else comes later (no stale position, no settle re-send after a clear).
    for _ in 0..20 {
        sites[0].tick();
    }
    let now = sites[0].ctl.host.now;
    assert!(pump(&hub, now, &mut sites[1]).is_empty());
    // A second clear with nothing shown sends nothing.
    set(&mut sites[0], None);
    assert!(pump(&hub, now, &mut sites[1]).is_empty());
}

#[test]
fn a_still_pointer_is_resent_once() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let a = sites[0].ctl.collab_site();
    let t0 = sites[0].ctl.host.now;
    set(&mut sites[0], Some(at(3.0)));
    assert_eq!(pump(&hub, t0, &mut sites[1]).len(), 1);
    let mut later = Vec::new();
    for _ in 0..40 {
        sites[0].tick();
        let now = sites[0].ctl.host.now;
        later.extend(pump(&hub, now, &mut sites[1]));
    }
    assert_eq!(later, [(a, Some(at(3.0)))], "one settle re-send");
}

#[test]
fn a_leaving_peer_pointer_is_cleared() {
    let hub = Hub::default();
    let mut sites = session(&hub, 3);
    let a = sites[0].ctl.collab_site();
    let now = sites[0].ctl.host.now;
    set(&mut sites[0], Some(at(8.0)));
    assert_eq!(pump(&hub, now, &mut sites[1]).len(), 1);
    // Site 2 never saw a pointer from site 1: no clear for it on site 1's leave.
    sites[0].ok(Command::Collab(CollabCommand::Leave));
    let mut refs: Vec<&mut Site> = sites[1..].iter_mut().collect();
    settle(&mut refs, &hub);
    let on1 = pointers(&sites[1].log);
    assert_eq!(on1.last(), Some(&(a, None)), "cleared on leave: {on1:?}");
    // Site 2 also got the pointer; it too gets the clear. Nobody else is cleared.
    let on2 = pointers(&sites[2].log);
    assert!(on2.iter().all(|(s, _)| *s == a), "{on2:?}");
}

#[test]
fn set_pointer_outside_a_session_is_a_no_op() {
    let hub = Hub::default();
    let mut lone = Site::on_hub(9, &hub);
    set(&mut lone, Some(at(1.0)));
    set(&mut lone, None);
}
