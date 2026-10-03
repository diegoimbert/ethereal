//! Sharing (docs/SHARING.md): the P2P host hub, invite links, joining, offline copies.
//!
//! - **Host** ([`host`]): `Start` creates or resumes the project's `share.json`, runs the hub
//!   (`ether_collab::share::hub`, the relay state machine in process) and joins it as the
//!   host site over a loopback link, then keeps a socket to the signaling service open so
//!   joiners can be introduced. Each introduction becomes a WebRTC pairing whose data
//!   channel is authenticated (`ether_collab::share::handshake`) and handed to the hub.
//! - **Joiner** ([`join`]): `OpenInvite` dials the host (signaling, ICE, handshake) and shows
//!   the invite; `AcceptInvite` starts the collab session over the data channel
//!   ([`port`]). Reconnects (drops, reopening an offline copy) dial again with the member
//!   key. The collab session itself (rebase, undo, catch-up, "(local copy)") is the
//!   existing one (`collab/mod.rs`), started with a per-session connector.
//! - `ShareEvent::State` is recomputed on every command and tick and sent when it changes.
//!
//! The platform pieces (signaling sockets, WebRTC endpoints) are injected as
//! `ShareServices` ([`EtherController::set_share_services`]); tests use
//! `ether_collab::share::fake`.

mod file;
mod host;
mod join;
mod port;

use ether_collab::share::file::ShareFile;
use ether_collab::share::{ShareServices, default_services};
use ether_core::protocol::collab::IceServer;
use ether_core::protocol::model::{Color, ProjectId};
use ether_core::protocol::share::{
    DEFAULT_SIGNAL_URL, ShareCommand, ShareEvent, ShareNotice, ShareRole, ShareState,
};
use ether_core::protocol::{Event, ReplyValue};

pub(crate) use file::clear_share_file;

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Name used when the UI never sent an identity.
const DEFAULT_NAME: &str = "Guest";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Prefs {
    pub resume_on_open: bool,
    pub auto_listen: bool,
    pub relay_only: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            resume_on_open: true,
            auto_listen: true,
            relay_only: false,
        }
    }
}

pub(crate) enum Mode {
    Off,
    Host(Box<host::HostRt>),
    Join(Box<join::JoinRt>),
}

/// Sharing state of the controller (one field on `EtherController`).
pub(crate) struct ShareCtl {
    services: Option<ShareServices>,
    name: Option<String>,
    color: Option<Color>,
    signal_url: Option<String>,
    invite_origin: Option<String>,
    prefs: Prefs,
    mode: Mode,
    /// Last `State` sent to the UI.
    emitted: Option<ShareState>,
    /// A project was opened: resume sharing or reconnect it at the next tick.
    opened: Option<ProjectId>,
}

impl Default for ShareCtl {
    fn default() -> Self {
        Self {
            services: None,
            name: None,
            color: None,
            signal_url: None,
            invite_origin: None,
            prefs: Prefs::default(),
            mode: Mode::Off,
            emitted: None,
            opened: None,
        }
    }
}

/// `https://x/signal` → `wss://x/signal/v1/rooms/<room>/<kind>`.
pub(crate) fn signal_socket_url(base: Option<&str>, room: &str, kind: &str) -> String {
    let base = base
        .unwrap_or(DEFAULT_SIGNAL_URL)
        .trim()
        .trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{base}/v1/rooms/{room}/{kind}")
}

/// The ICE servers to hand to a peer connection: the settings' list, else the signaling
/// service's; relay-only keeps the TURN servers only.
pub(crate) fn peer_ice(
    override_: Option<&[IceServer]>,
    advertised: &[IceServer],
    relay_only: bool,
) -> Vec<IceServer> {
    let list = override_.unwrap_or(advertised);
    list.iter()
        .filter_map(|s| {
            if !relay_only {
                return Some(s.clone());
            }
            let urls: Vec<String> = s
                .urls
                .iter()
                .filter(|u| u.starts_with("turn:") || u.starts_with("turns:"))
                .cloned()
                .collect();
            (!urls.is_empty()).then(|| IceServer { urls, ..s.clone() })
        })
        .collect()
}

/// A display name from a peer: trimmed, at most 64 chars, never empty.
pub(crate) fn clean_name(name: &str) -> String {
    let n: String = name.trim().chars().take(64).collect();
    if n.is_empty() {
        DEFAULT_NAME.to_string()
    } else {
        n
    }
}

pub(crate) fn notice(out: &mut dyn MessageSink, notice: ShareNotice) {
    event(
        out,
        Event::Share {
            event: ShareEvent::Notice { notice },
        },
    );
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// The platform's sharing services (signaling sockets, WebRTC endpoints; node
    /// `p2p-transport`), or in-memory fakes in tests (`ether_collab::share::fake`). Default:
    /// [`default_services`], the real ones (native: tungstenite + rustls signaling and str0m
    /// data channels, used as is by the desktop app and `ether-server`; web: the Worker's
    /// `WebSocket` and the UI's peer connections behind the share port). So hosts don't call
    /// this; the signaling URL defaults to `DEFAULT_SIGNAL_URL` and the ICE servers to the
    /// ones the service advertises (Cloudflare STUN), both overridable (`SetServers`,
    /// `Collab::SetIceServers`).
    pub fn set_share_services(&mut self, services: ShareServices) {
        self.share.services = Some(services);
    }

    pub(crate) fn share_services(&mut self) -> &mut ShareServices {
        self.share.services.get_or_insert_with(default_services)
    }

    pub(crate) fn share_name(&self) -> String {
        self.share
            .name
            .clone()
            .unwrap_or_else(|| DEFAULT_NAME.to_string())
    }

    pub(crate) fn share_app(&self) -> String {
        format!("Ethereal {}", self.config.app_version)
    }

    /// A joiner with a listen link: document edits are refused (docs/SHARING.md §2.3).
    pub(crate) fn share_view_only(&self) -> bool {
        matches!(&self.share.mode, Mode::Join(j) if j.view_only())
    }

    pub(crate) fn share_command(
        &mut self,
        c: &ShareCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let now = self.host.now_ms();
        match c {
            ShareCommand::Get => {
                self.share.emitted = None;
            }
            ShareCommand::SetIdentity { name, color } => {
                let name: String = name.trim().to_string();
                let n = name.chars().count();
                if n == 0 || n > 64 {
                    return Err(invalid("names are 1-64 characters"));
                }
                self.share.name = Some(name);
                self.share.color = *color;
            }
            ShareCommand::SetServers {
                signal_url,
                invite_origin,
            } => {
                let check = |u: &Option<String>| match u.as_deref().map(str::trim) {
                    None | Some("") => Ok(None),
                    Some(u) if u.starts_with("https://") || u.starts_with("http://") => {
                        Ok(Some(u.trim_end_matches('/').to_string()))
                    }
                    Some(_) => Err(invalid("server URLs start with https:// or http://")),
                };
                let signal = check(signal_url)?;
                let origin = check(invite_origin)?;
                self.share.signal_url = signal.filter(|u| u != DEFAULT_SIGNAL_URL);
                self.share.invite_origin = origin;
            }
            ShareCommand::SetPreferences {
                resume_on_open,
                auto_listen,
                relay_only,
            } => {
                self.share.prefs = Prefs {
                    resume_on_open: *resume_on_open,
                    auto_listen: *auto_listen,
                    relay_only: *relay_only,
                };
            }
            ShareCommand::Start => self.share_start(now, out)?,
            ShareCommand::Stop => self.share_stop(out)?,
            ShareCommand::ResetLink { role } => self.share_reset_link(*role)?,
            ShareCommand::RemoveParticipant { member } => self.share_remove(member)?,
            ShareCommand::SetParticipantRole { member, role } => {
                self.share_set_role(member, *role)?
            }
            ShareCommand::OpenInvite { link } => self.share_open_invite(link, now, out)?,
            ShareCommand::AcceptInvite => self.share_accept(now, out)?,
            ShareCommand::Leave => self.share_leave(out)?,
            ShareCommand::Reconnect { project } => self.share_reconnect(*project, now, out)?,
            ShareCommand::Detach { project } => self.share_detach(*project, out)?,
            // p2p-transport (web UI endpoint).
            ShareCommand::PeerSignal { .. } => {
                return Err(unsupported(
                    "the UI peer endpoint is not implemented yet (docs/SHARING.md)",
                ));
            }
        }
        self.share_emit_state(out);
        Ok(ReplyValue::Unit)
    }

    /// Controller tick (after the collab tick).
    pub(crate) fn share_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        if let Some(pid) = self.share.opened.take() {
            self.share_after_open(pid, now, out);
        }
        match &self.share.mode {
            Mode::Off => {}
            Mode::Host(_) => self.share_host_tick(now, out),
            Mode::Join(_) => self.share_join_tick(now, out),
        }
        if let Some((project, name)) = self.collab_take_backup_kept() {
            notice(out, ShareNotice::LocalCopyKept { project, name });
        }
        self.share_emit_state(out);
    }

    /// A project became the open document (`load_project`): hosting or a joined session of
    /// another project pauses; this one resumes sharing or reconnects at the next tick.
    pub(crate) fn share_project_loaded(&mut self, pid: ProjectId, out: &mut dyn MessageSink) {
        match &self.share.mode {
            Mode::Host(h) if h.project != pid => self.share_host_pause(out),
            Mode::Join(j) if j.project.is_some_and(|p| p != pid) => {
                self.share_join_end(out, false);
            }
            _ => {}
        }
        if matches!(self.share.mode, Mode::Off) {
            self.share.opened = Some(pid);
        }
    }

    fn share_after_open(&mut self, pid: ProjectId, now: u64, out: &mut dyn MessageSink) {
        if !matches!(self.share.mode, Mode::Off)
            || self.doc.as_ref().map(|d| d.project.id) != Some(pid)
        {
            return;
        }
        match file::read_share(&mut self.store, pid) {
            Some(ShareFile::Host(h)) if h.resume && self.share.prefs.resume_on_open => {
                let _ = self.share_start(now, out);
            }
            Some(ShareFile::Copy(c)) if !c.key.is_empty() => {
                let _ = self.share_join_copy(pid, c, now, out);
            }
            _ => {}
        }
    }

    fn share_state(&self) -> ShareState {
        match &self.share.mode {
            Mode::Off => ShareState::Off,
            Mode::Host(h) => self.share_host_state(h),
            Mode::Join(j) => self.share_join_state(j),
        }
    }

    pub(crate) fn share_emit_state(&mut self, out: &mut dyn MessageSink) {
        let state = self.share_state();
        if self.share.emitted.as_ref() != Some(&state) {
            self.share.emitted = Some(state.clone());
            event(
                out,
                Event::Share {
                    event: ShareEvent::State { state },
                },
            );
        }
    }

    fn share_detach(&mut self, project: ProjectId, out: &mut dyn MessageSink) -> CmdResult<()> {
        if let Mode::Join(j) = &self.share.mode
            && j.project == Some(project)
        {
            self.share_join_end(out, true);
        }
        match file::read_share(&mut self.store, project) {
            Some(ShareFile::Copy(_)) => {}
            Some(ShareFile::Host(_)) => {
                return Err(invalid_state(
                    "this is your shared project: use Stop sharing instead",
                ));
            }
            None => return Ok(()),
        }
        clear_share_file(&mut self.store, project);
        self.emit_list_changed(out);
        Ok(())
    }

    /// The role of a joined session, for `ShareRole` checks.
    #[allow(dead_code)]
    pub(crate) fn share_role(&self) -> Option<ShareRole> {
        match &self.share.mode {
            Mode::Join(j) => Some(j.role),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_urls() {
        assert_eq!(
            signal_socket_url(None, "R", "host"),
            "wss://etherealws.pages.dev/signal/v1/rooms/R/host"
        );
        assert_eq!(
            signal_socket_url(Some("http://localhost:8787/"), "R", "join"),
            "ws://localhost:8787/v1/rooms/R/join"
        );
    }

    #[test]
    fn relay_only_keeps_turn() {
        let stun = IceServer {
            urls: vec!["stun:s:3478".into()],
            username: None,
            credential: None,
        };
        let turn = IceServer {
            urls: vec!["stun:t:3478".into(), "turn:t:3478?transport=udp".into()],
            username: Some("u".into()),
            credential: Some("c".into()),
        };
        let all = [stun.clone(), turn];
        assert_eq!(peer_ice(None, &all, false).len(), 2);
        let relay = peer_ice(None, &all, true);
        assert_eq!(relay.len(), 1);
        assert_eq!(relay[0].urls, ["turn:t:3478?transport=udp"]);
        assert_eq!(
            peer_ice(Some(&[stun]), &all, false).len(),
            1,
            "settings win"
        );
        assert_eq!(clean_name("  "), DEFAULT_NAME);
    }
}
