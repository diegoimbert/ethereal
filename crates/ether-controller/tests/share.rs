//! Sharing (docs/SHARING.md §12, node `share-engine`): real controllers sharing and joining
//! through the fake signaling service and in-memory WebRTC endpoints
//! (`ether_collab::share::fake`). The hub is the real relay state machine inside the host.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_collab::share::ShareServices;
use ether_collab::share::fake::FakeNet;
use ether_collab::share::file::{SHARE_FILE, ShareFile};
use ether_controller::store::ProjectStore;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::share::*;
use ether_core::protocol::social::ChatCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use proptest::prelude::*;
use support::*;

const HOST_SEED: u64 = 0x1001;

/// A site with the fake sharing services and an identity.
fn site(net: &FakeNet, seed: u64, name: &str) -> Site {
    // Relay sessions are not used here.
    let unused = Hub::default();
    let mut s = Site::on_hub(seed, &unused);
    s.ctl.set_share_services(ShareServices {
        signal: net.signal_connector(),
        peers: net.endpoint(&format!("sha-256 {seed:04X}:AA")),
    });
    share(
        &mut s,
        ShareCommand::SetIdentity {
            name: name.into(),
            color: None,
        },
    );
    s
}

fn share(s: &mut Site, c: ShareCommand) -> ReplyValue {
    s.ok(Command::Share(c))
}

fn share_err(s: &mut Site, c: ShareCommand) -> ErrorCode {
    let out = s.send(Command::Share(c));
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// The last `ShareState` the UI got.
fn state(s: &Site) -> ShareState {
    s.log
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::Event(Event::Share {
                event: ShareEvent::State { state },
            }) => Some(state.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn notices(s: &Site) -> Vec<ShareNotice> {
    s.log
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Share {
                event: ShareEvent::Notice { notice },
            }) => Some(notice.clone()),
            _ => None,
        })
        .collect()
}

fn run(sites: &mut [&mut Site], rounds: usize) {
    for _ in 0..rounds {
        for s in sites.iter_mut() {
            s.tick();
        }
    }
}

/// Tick until every site's pending edits are sequenced and nothing changes (bounded).
fn settle(sites: &mut [&mut Site]) {
    run(sites, 30);
    for _ in 0..100 {
        if sites.iter().all(|s| s.ctl.collab_pending() == 0) {
            run(sites, 10);
            return;
        }
        run(sites, 10);
    }
    panic!(
        "did not settle: {:?}",
        sites.iter().map(|s| state(s)).collect::<Vec<_>>()
    );
}

fn links(s: &Site) -> (String, String) {
    match state(s) {
        ShareState::Hosting {
            edit_link: Some(e),
            listen_link: Some(l),
            ..
        } => (e, l),
        other => panic!("not hosting with links: {other:?}"),
    }
}

/// Diego shares "Song".
fn host(net: &FakeNet) -> (Site, ProjectId) {
    let mut h = site(net, HOST_SEED, "Diego");
    let pid = h.create_project("Song");
    share(&mut h, ShareCommand::Start);
    run(&mut [&mut h], 5);
    assert!(matches!(
        state(&h),
        ShareState::Hosting {
            signal: SignalStatus::Online,
            ..
        }
    ));
    (h, pid)
}

fn ready(j: &Site) -> InvitePreview {
    match state(j) {
        ShareState::Joining {
            stage: JoinStage::Ready { invite },
        } => invite,
        other => panic!("not ready: {other:?}"),
    }
}

/// `j` opens `link`, sees the invite and joins.
fn join(h: &mut Site, j: &mut Site, link: &str) {
    share(j, ShareCommand::OpenInvite { link: link.into() });
    run(&mut [&mut *h, &mut *j], 10);
    ready(j);
    share(j, ShareCommand::AcceptInvite);
    settle(&mut [&mut *h, &mut *j]);
    assert!(joined_online(j), "{:?}", state(j));
}

fn joined_online(j: &Site) -> bool {
    matches!(
        state(j),
        ShareState::Joined {
            link: HostLink::Online,
            ..
        }
    )
}

fn add_track(s: &mut Site) -> TrackId {
    let id = s.id();
    s.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn share_file(s: &mut Site, pid: ProjectId) -> Option<ShareFile> {
    let bytes = s.ctl.store.read(pid, SHARE_FILE).ok()?;
    (!bytes.is_empty()).then(|| serde_json::from_slice(&bytes).expect("share.json parses"))
}

fn hosting_participants(h: &Site) -> Vec<Participant> {
    match state(h) {
        ShareState::Hosting { participants, .. } => participants,
        other => panic!("{other:?}"),
    }
}

// ─── Host + joiners ─────────────────────────────────────────────────────────────────────

#[test]
fn a_host_and_two_joiners_converge() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    add_track(&mut h);
    let (edit, _) = links(&h);
    assert!(
        edit.starts_with("https://etherealws.pages.dev/join/"),
        "{edit}"
    );
    let mut ada = site(&net, 0x2002, "Ada");
    share(&mut ada, ShareCommand::OpenInvite { link: edit.clone() });
    run(&mut [&mut h, &mut ada], 10);
    let invite = ready(&ada);
    assert_eq!(invite.host.name, "Diego");
    assert_eq!(invite.project_name, "Song");
    assert_eq!(invite.project, pid);
    assert_eq!(invite.role, ShareRole::Edit);
    assert!(!invite.local_copy);
    // The host records a member only on Accept.
    assert_eq!(hosting_participants(&h).len(), 1);
    share(&mut ada, ShareCommand::AcceptInvite);
    settle(&mut [&mut h, &mut ada]);
    assert!(joined_online(&ada));
    assert_eq!(ada.project().id, pid);
    assert!(notices(&h).contains(&ShareNotice::ParticipantJoined {
        name: "Ada".into(),
        color: hosting_participants(&h)[1].color,
    }));
    let mut tom = site(&net, 0x3003, "Tom");
    join(&mut h, &mut tom, &edit);
    // Edits from everyone, concurrently.
    for _ in 0..3 {
        add_track(&mut h);
        add_track(&mut ada);
        add_track(&mut tom);
    }
    settle(&mut [&mut h, &mut ada, &mut tom]);
    assert_eq!(h.tracks().len(), 10);
    assert_converged(&[&h, &ada, &tom]);
    // Participants: host first, members with their sites, all online.
    let ps = hosting_participants(&h);
    assert_eq!(ps.len(), 3);
    assert_eq!(ps[0].role, ParticipantRole::Host);
    assert!(ps[0].you && ps.iter().all(|p| p.online && p.site.is_some()));
    assert_eq!(ps[1].name, "Ada");
    assert_eq!(ps[1].role, ParticipantRole::Edit);
    // The joiners see the host first, themselves, and each other.
    match state(&ada) {
        ShareState::Joined {
            participants,
            role: ShareRole::Edit,
            project,
            ..
        } => {
            assert_eq!(project, pid);
            assert_eq!(participants[0].name, "Diego");
            assert_eq!(participants[0].site, Some(SiteId(HOST_SEED)));
            assert!(participants[1].you);
            assert!(participants.iter().any(|p| p.name == "Tom"));
        }
        other => panic!("{other:?}"),
    }
    // The copies are recorded as offline copies of Diego's project.
    match share_file(&mut ada, pid) {
        Some(ShareFile::Copy(c)) => {
            assert_eq!(c.host_name, "Diego");
            assert_eq!(c.role, ShareRole::Edit);
            assert!(!c.key.is_empty());
        }
        other => panic!("{other:?}"),
    }
    // `sites` follows the session (throttled writes).
    h.advance(3_000);
    h.tick();
    match share_file(&mut h, pid) {
        Some(ShareFile::Host(f)) => {
            assert_eq!(f.members.len(), 2);
            assert!(f.resume);
            assert!(f.sites.contains_key(&SiteId(0x2002)), "{:?}", f.sites);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_signaling_service_never_sees_the_project() {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    // Ada closed her socket once the data channel was up; the host's stays.
    assert_eq!(net.sockets(), 1);
    assert_eq!(net.links(), 1);
    assert_eq!(
        net.doors(&room_of(&edit)),
        3,
        "two links and Ada's member key"
    );
}

fn room_of(link: &str) -> String {
    ether_collab::share::invite::Invite::parse(link)
        .unwrap()
        .room
}

#[test]
fn listen_links_are_view_only_but_can_chat() {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (_, listen) = links(&h);
    let mut tom = site(&net, 0x3003, "Tom");
    share(&mut tom, ShareCommand::OpenInvite { link: listen });
    run(&mut [&mut h, &mut tom], 10);
    assert_eq!(ready(&tom).role, ShareRole::Listen);
    share(&mut tom, ShareCommand::AcceptInvite);
    settle(&mut [&mut h, &mut tom]);
    assert!(matches!(
        state(&tom),
        ShareState::Joined {
            role: ShareRole::Listen,
            ..
        }
    ));
    // Edits are refused before they touch the document.
    let id = tom.id();
    let out = tom.send(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => {
            assert_eq!(error.code, ErrorCode::InvalidState);
            assert!(error.message.contains("view only"));
        }
        other => panic!("{other:?}"),
    }
    assert!(tom.tracks().is_empty());
    // Chat works.
    let msg = tom.id();
    tom.ok(Command::Chat(ChatCommand::Send {
        id: msg,
        text: "nice groove".into(),
    }));
    settle(&mut [&mut h, &mut tom]);
    assert_eq!(h.project().chat.len(), 1);
    assert!(joined_online(&tom), "chat did not close the link");
    // The host's edits reach the listener.
    add_track(&mut h);
    settle(&mut [&mut h, &mut tom]);
    assert_converged(&[&h, &tom]);
    assert_eq!(hosting_participants(&h)[1].role, ParticipantRole::Listen);
}

// ─── Links and members ──────────────────────────────────────────────────────────────────

#[test]
fn reset_link_keeps_members() {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (old, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &old);
    share(
        &mut h,
        ShareCommand::ResetLink {
            role: ShareRole::Edit,
        },
    );
    let (new, _) = links(&h);
    assert_ne!(new, old);
    // A member's connection drops: she comes back with her member key.
    net.kill_links();
    run(&mut [&mut h, &mut ada], 3);
    assert!(matches!(
        state(&ada),
        ShareState::Joined {
            link: HostLink::Connecting { .. },
            ..
        }
    ));
    settle(&mut [&mut h, &mut ada]);
    assert!(joined_online(&ada), "{:?}", state(&ada));
    add_track(&mut ada);
    settle(&mut [&mut h, &mut ada]);
    assert_converged(&[&h, &ada]);
    // The old link no longer works; the new one does.
    let mut tom = site(&net, 0x3003, "Tom");
    share(&mut tom, ShareCommand::OpenInvite { link: old });
    run(&mut [&mut h, &mut tom], 10);
    assert!(matches!(
        state(&tom),
        ShareState::Joining {
            stage: JoinStage::Failed {
                reason: JoinFailure::InvalidInvite,
                ..
            }
        }
    ));
    join(&mut h, &mut tom, &new);
}

#[test]
fn removing_a_member_ends_its_sharing() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    let member = hosting_participants(&h)[1].member.clone().unwrap();
    assert_eq!(
        share_err(
            &mut h,
            ShareCommand::RemoveParticipant {
                member: "nobody".into()
            }
        ),
        ErrorCode::InvalidArgument
    );
    share(&mut h, ShareCommand::RemoveParticipant { member });
    settle(&mut [&mut h, &mut ada]);
    assert_eq!(state(&ada), ShareState::Off);
    assert!(notices(&ada).contains(&ShareNotice::SharingEnded {
        host_name: "Diego".into()
    }));
    // Her copy stays, marked "sharing ended".
    assert_eq!(ada.project().id, pid);
    match share_file(&mut ada, pid) {
        Some(ShareFile::Copy(c)) => assert!(c.key.is_empty()),
        other => panic!("{other:?}"),
    }
    assert_eq!(hosting_participants(&h).len(), 1);
    assert_eq!(
        share_err(&mut ada, ShareCommand::Reconnect { project: pid }),
        ErrorCode::InvalidState
    );
    // A private copy.
    share(&mut ada, ShareCommand::Detach { project: pid });
    assert!(share_file(&mut ada, pid).is_none());
}

#[test]
fn a_role_change_applies_at_once() {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    let member = hosting_participants(&h)[1].member.clone().unwrap();
    share(
        &mut h,
        ShareCommand::SetParticipantRole {
            member,
            role: ShareRole::Listen,
        },
    );
    settle(&mut [&mut h, &mut ada]);
    assert!(
        matches!(
            state(&ada),
            ShareState::Joined {
                role: ShareRole::Listen,
                link: HostLink::Online,
                ..
            }
        ),
        "{:?}",
        state(&ada)
    );
    let id = ada.id();
    let out = ada.send(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    assert!(matches!(
        out.last(),
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { .. },
            ..
        }))
    ));
}

#[test]
fn stop_sharing_revokes_everything() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, listen) = links(&h);
    let room = room_of(&edit);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    let mut tom = site(&net, 0x3003, "Tom");
    join(&mut h, &mut tom, &listen);
    share(&mut h, ShareCommand::Stop);
    assert_eq!(state(&h), ShareState::Off);
    assert!(!net.room_exists(&room));
    assert!(share_file(&mut h, pid).is_none());
    settle(&mut [&mut h, &mut ada, &mut tom]);
    for s in [&ada, &tom] {
        assert_eq!(state(s), ShareState::Off);
        assert!(notices(s).contains(&ShareNotice::SharingEnded {
            host_name: "Diego".into()
        }));
        assert_eq!(s.project().id, pid, "an offline copy stays");
    }
    // The host keeps editing a normal project; a new share is a new room.
    add_track(&mut h);
    share(&mut h, ShareCommand::Start);
    run(&mut [&mut h], 5);
    let (edit2, _) = links(&h);
    assert_ne!(room_of(&edit2), room);
}

// ─── Lifecycle ──────────────────────────────────────────────────────────────────────────

#[test]
fn host_offline_and_back() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    add_track(&mut h);
    settle(&mut [&mut h, &mut ada]);
    // Diego opens another project: sharing pauses.
    let other = h.create_project("Other");
    run(&mut [&mut h, &mut ada], 60);
    assert_eq!(state(&h), ShareState::Off);
    assert!(matches!(
        state(&ada),
        ShareState::Joined {
            link: HostLink::HostOffline { .. },
            ..
        }
    ));
    assert!(notices(&ada).contains(&ShareNotice::HostOffline {
        host_name: "Diego".into()
    }));
    // Ada keeps working on her copy (pending until Diego is back).
    add_track(&mut ada);
    assert_eq!(ada.ctl.collab_pending(), 1);
    assert_ne!(other, pid);
    // Diego reopens Song: sharing resumes (same links), Ada reconnects.
    h.ok(Command::Project(ProjectCommand::Open { id: pid }));
    settle(&mut [&mut h, &mut ada]);
    assert_eq!(links(&h).0, edit, "same links");
    assert!(joined_online(&ada), "{:?}", state(&ada));
    assert!(notices(&ada).contains(&ShareNotice::HostBack {
        host_name: "Diego".into()
    }));
    assert_eq!(h.tracks().len(), 2, "Ada's offline edit synced");
    assert_converged(&[&h, &ada]);
}

/// Real endpoints may open the joiner's end of the data channel first; the joiner then
/// closes its signaling socket and the host sees `PeerLeft` before its own `Connected`
/// (share-integration found this in the browser e2e: "the connection to the host was lost").
#[test]
fn a_joiner_leaving_signaling_before_the_host_sees_the_channel_still_joins() {
    let net = FakeNet::new();
    net.set_answerer_lag(true);
    let (mut h, _) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    add_track(&mut ada);
    settle(&mut [&mut h, &mut ada]);
    assert_eq!(h.tracks().len(), 1);
    assert_converged(&[&h, &ada]);
}

#[test]
fn a_restarted_host_starts_a_new_epoch_and_dedupes_resends() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    for _ in 0..3 {
        add_track(&mut ada);
    }
    settle(&mut [&mut h, &mut ada]);
    h.ok(Command::Project(ProjectCommand::Save));
    // The host app restarts: a new controller on the same store.
    let store = h.ctl.store.clone();
    drop(h);
    // Its data channels die with it (ICE consent).
    net.kill_links();
    run(&mut [&mut ada], 5);
    add_track(&mut ada);
    let mut h = site(&net, HOST_SEED, "Diego");
    h.ctl.store = store;
    h.ok(Command::Project(ProjectCommand::Open { id: pid }));
    settle(&mut [&mut h, &mut ada]);
    assert!(matches!(state(&h), ShareState::Hosting { .. }), "resumed");
    assert!(joined_online(&ada), "{:?}", state(&ada));
    assert_eq!(h.tracks().len(), 4, "nothing lost, nothing applied twice");
    assert_converged(&[&h, &ada]);
}

#[test]
fn a_reopened_offline_copy_reconnects_and_keeps_offline_work() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    add_track(&mut h);
    settle(&mut [&mut h, &mut ada]);
    share(&mut ada, ShareCommand::Leave);
    assert_eq!(state(&ada), ShareState::Off);
    assert_eq!(ada.project().id, pid);
    // Offline work on the copy (saved), and new work on the master.
    add_track(&mut ada);
    ada.ok(Command::Project(ProjectCommand::Save));
    add_track(&mut h);
    run(&mut [&mut h], 5);
    // Reconnect from Recents: no confirmation screen (member key).
    share(&mut ada, ShareCommand::Reconnect { project: pid });
    settle(&mut [&mut h, &mut ada]);
    assert!(joined_online(&ada), "{:?}", state(&ada));
    assert_converged(&[&h, &ada]);
    assert_eq!(h.tracks().len(), 2);
    let kept = notices(&ada).into_iter().find_map(|n| match n {
        ShareNotice::LocalCopyKept { name, .. } => Some(name),
        _ => None,
    });
    assert_eq!(kept.as_deref(), Some("Song (local copy)"));
}

#[test]
fn opening_an_offline_copy_reconnects_automatically() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    let mine = ada.create_project("Mine");
    run(&mut [&mut h, &mut ada], 5);
    assert_eq!(
        state(&ada),
        ShareState::Off,
        "another project: the copy is offline"
    );
    assert_ne!(mine, pid);
    add_track(&mut h);
    ada.ok(Command::Project(ProjectCommand::Open { id: pid }));
    settle(&mut [&mut h, &mut ada]);
    assert!(joined_online(&ada), "{:?}", state(&ada));
    assert_converged(&[&h, &ada]);
}

#[test]
fn joining_while_the_host_is_offline_waits() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    let (edit, _) = links(&h);
    h.create_project("Other");
    run(&mut [&mut h], 3);
    let mut ada = site(&net, 0x2002, "Ada");
    share(&mut ada, ShareCommand::OpenInvite { link: edit });
    run(&mut [&mut h, &mut ada], 10);
    assert!(matches!(
        state(&ada),
        ShareState::Joining {
            stage: JoinStage::HostOffline { .. }
        }
    ));
    h.ok(Command::Project(ProjectCommand::Open { id: pid }));
    run(&mut [&mut h, &mut ada], 10);
    assert_eq!(ready(&ada).project, pid);
    // "Not now".
    share(&mut ada, ShareCommand::Leave);
    assert_eq!(state(&ada), ShareState::Off);
    run(&mut [&mut h, &mut ada], 10);
    assert_eq!(hosting_participants(&h).len(), 1, "declined: no trace");
}

// ─── Failures ───────────────────────────────────────────────────────────────────────────

fn failed(s: &Site) -> JoinFailure {
    match state(s) {
        ShareState::Joining {
            stage: JoinStage::Failed { reason, .. },
        } => reason,
        other => panic!("{other:?}"),
    }
}

#[test]
fn bad_links_and_unreachable_hosts_fail_cleanly() {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    share(
        &mut ada,
        ShareCommand::OpenInvite {
            link: "https://x.test/song".into(),
        },
    );
    assert_eq!(failed(&ada), JoinFailure::BadLink);
    let v2 = edit.replace("#1", "#2");
    share(&mut ada, ShareCommand::OpenInvite { link: v2 });
    assert_eq!(failed(&ada), JoinFailure::Version);
    net.set_unreachable(true);
    share(&mut ada, ShareCommand::OpenInvite { link: edit.clone() });
    run(&mut [&mut h, &mut ada], 10);
    assert_eq!(failed(&ada), JoinFailure::Unreachable);
    net.set_unreachable(false);
    // A tampering service (another host fingerprint) cannot get anyone in.
    net.set_tamper(true);
    share(&mut ada, ShareCommand::OpenInvite { link: edit.clone() });
    run(&mut [&mut h, &mut ada], 10);
    assert_eq!(failed(&ada), JoinFailure::InvalidInvite);
    assert_eq!(hosting_participants(&h).len(), 1);
    net.set_tamper(false);
    join(&mut h, &mut ada, &edit);
}

#[test]
fn the_host_shows_when_the_service_is_unreachable() {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    net.set_down(true);
    run(&mut [&mut h, &mut ada], 5);
    match state(&h) {
        ShareState::Hosting {
            signal: SignalStatus::Offline { .. },
            edit_link: None,
            ..
        } => {}
        other => panic!("{other:?}"),
    }
    // People already here stay connected.
    add_track(&mut ada);
    settle(&mut [&mut h, &mut ada]);
    assert_converged(&[&h, &ada]);
    net.set_down(false);
    run(&mut [&mut h, &mut ada], 100);
    assert_eq!(links(&h).0, edit);
}

#[test]
fn commands_check_their_state() {
    let net = FakeNet::new();
    let mut s = site(&net, 0x2002, "Ada");
    assert_eq!(
        share_err(&mut s, ShareCommand::Start),
        ErrorCode::InvalidState
    );
    assert_eq!(
        share_err(
            &mut s,
            ShareCommand::SetIdentity {
                name: " ".into(),
                color: None
            }
        ),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        share_err(&mut s, ShareCommand::Stop),
        ErrorCode::InvalidState
    );
    assert_eq!(
        share_err(&mut s, ShareCommand::AcceptInvite),
        ErrorCode::InvalidState
    );
    assert_eq!(
        share_err(
            &mut s,
            ShareCommand::ResetLink {
                role: ShareRole::Edit
            }
        ),
        ErrorCode::InvalidState
    );
    share(&mut s, ShareCommand::Leave);
    share(
        &mut s,
        ShareCommand::SetServers {
            signal_url: Some("https://signal.example.org".into()),
            invite_origin: Some("http://localhost:5173".into()),
        },
    );
    share(
        &mut s,
        ShareCommand::SetPreferences {
            resume_on_open: false,
            auto_listen: false,
            relay_only: false,
        },
    );
    assert_eq!(
        share_err(
            &mut s,
            ShareCommand::SetServers {
                signal_url: Some("ftp://x".into()),
                invite_origin: None,
            }
        ),
        ErrorCode::InvalidArgument
    );
    let pid = s.create_project("Song");
    share(&mut s, ShareCommand::Start);
    let ShareState::Hosting { project, .. } = state(&s) else {
        panic!("{:?}", state(&s))
    };
    assert_eq!(project, pid);
    assert_eq!(
        share_err(&mut s, ShareCommand::Leave),
        ErrorCode::InvalidState
    );
    // Not resumed on open (preference off).
    s.create_project("Other");
    s.ok(Command::Project(ProjectCommand::Open { id: pid }));
    run(&mut [&mut s], 5);
    assert_eq!(state(&s), ShareState::Off);
}

#[test]
fn self_hosted_services_travel_in_links() {
    let net = FakeNet::new();
    let mut h = site(&net, HOST_SEED, "Diego");
    share(
        &mut h,
        ShareCommand::SetServers {
            signal_url: Some("https://signal.example.org/".into()),
            invite_origin: Some("http://localhost:5173".into()),
        },
    );
    h.create_project("Song");
    share(&mut h, ShareCommand::Start);
    run(&mut [&mut h], 5);
    let (edit, _) = links(&h);
    assert!(edit.starts_with("http://localhost:5173/join/"), "{edit}");
    assert!(
        edit.contains("?s=https%3A%2F%2Fsignal.example.org#1"),
        "{edit}"
    );
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
}

// ─── Copies are private ─────────────────────────────────────────────────────────────────

#[test]
fn save_as_and_duplicate_never_copy_share_json() {
    let net = FakeNet::new();
    let (mut h, pid) = host(&net);
    assert!(matches!(share_file(&mut h, pid), Some(ShareFile::Host(_))));
    let dup = h.ids.next_project_id(T0);
    h.ok(Command::Project(ProjectCommand::Duplicate {
        id: pid,
        new_id: dup,
        name: "Song copy".into(),
    }));
    assert!(share_file(&mut h, dup).is_none());
    let save_as = h.ids.next_project_id(T0);
    h.ok(Command::Project(ProjectCommand::SaveAs {
        new_id: save_as,
        name: "Song 2".into(),
    }));
    assert!(share_file(&mut h, save_as).is_none());
    // The open project is now the private one: sharing of Song pauses.
    run(&mut [&mut h], 3);
    assert_eq!(state(&h), ShareState::Off);
    assert!(matches!(
        share_file(&mut h, pid),
        Some(ShareFile::Host(f)) if f.resume
    ));
}

// ─── Convergence through the hub (the collab property test, P2P) ────────────────────────

fn random_edit(s: &mut Site, action: u8, r1: u64, r2: u64) {
    let tracks = s.tracks();
    let clips: Vec<ClipId> = s.project().clips.keys().copied().collect();
    fn pick<T>(v: &[T], r: u64) -> Option<usize> {
        (!v.is_empty()).then(|| (r % v.len() as u64) as usize)
    }
    let cmd = match action % 6 {
        0 | 1 => {
            add_track(s);
            return;
        }
        2 => match pick(&tracks, r1) {
            Some(i) => Command::Track(TrackCommand::Delete { id: tracks[i] }),
            None => return,
        },
        3 | 4 => match pick(&tracks, r1) {
            Some(i) => Command::Clip(ClipCommand::CreateMidi {
                id: s.id(),
                track: tracks[i],
                start: Beats((r2 % 16) as f64),
                length: Beats(2.0),
                name: None,
            }),
            None => return,
        },
        _ => match pick(&clips, r1) {
            Some(i) => Command::Clip(ClipCommand::Delete {
                ids: vec![clips[i]],
            }),
            None => return,
        },
    };
    let _ = s.send(cmd);
}

fn simulate(steps: &[(usize, u8, u64, u64)]) {
    let net = FakeNet::new();
    let (mut h, _) = host(&net);
    let (edit, _) = links(&h);
    let mut ada = site(&net, 0x2002, "Ada");
    join(&mut h, &mut ada, &edit);
    let mut tom = site(&net, 0x3003, "Tom");
    join(&mut h, &mut tom, &edit);
    let mut sites = [h, ada, tom];
    for &(i, action, r1, r2) in steps {
        let i = i % 3;
        match action % 10 {
            // Time passes for one site (it sends, receives, reconnects).
            6 | 7 => {
                sites[i].tick();
            }
            // Everyone ticks.
            8 => {
                for s in sites.iter_mut() {
                    s.tick();
                }
            }
            // Every data channel drops (pending edits are resent).
            9 if r2.is_multiple_of(5) => net.kill_links(),
            9 => {}
            a => random_edit(&mut sites[i], a, r1, r2),
        }
    }
    let [h, ada, tom] = &mut sites;
    settle(&mut [h, ada, tom]);
    assert!(joined_online(ada) && joined_online(tom));
    assert_converged(&[h, ada, tom]);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(16),
        ..ProptestConfig::default()
    })]

    #[test]
    fn replicas_converge_through_the_hub(steps in proptest::collection::vec(
        (0usize..3, 0u8..10, any::<u64>(), any::<u64>()), 10..60)) {
        simulate(&steps);
    }
}
