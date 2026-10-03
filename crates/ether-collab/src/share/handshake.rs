//! The data-channel handshake (docs/SHARING.md §4.3): `PeerHandshake` text frames before any
//! `CollabMessage`, with key proofs bound to both DTLS fingerprints.
//!
//! ```text
//! joiner → host  Hello   { credential: Link | Member, proof(K, fp_joiner, fp_host), name, .. }
//! host           admit(): tries K_edit, K_listen (Link) or K_member (Member) → role
//! host → joiner  Welcome { host_proof(K, fp_host, fp_joiner), role, member, member_key, .. }
//!                or Refused { reason } (and closes)
//! joiner         verify_welcome() (else HostNotVerified: never joins)
//! joiner → host  Accept   → CollabMessage frames both ways
//! ```
//!
//! Pure functions; the timers (10 s to `Hello`, 60 s to `Accept`) and the state live in the
//! controller (`ether-controller/src/share`).

use ether_protocol::share::{
    Credential, JoinFailure, MemberId, PeerHandshake, SHARE_PROTOCOL_VERSION, ShareRole,
};

use super::invite::LinkKey;
use super::keys;
use crate::wire::{COLLAB_PROTOCOL_VERSION, WireFrame};

/// The host waits this long for `Hello` once the data channel is open.
pub const HELLO_TIMEOUT_MS: u64 = 10_000;
/// The host waits this long for `Accept` after `Welcome` (the user reads the join screen).
pub const ACCEPT_TIMEOUT_MS: u64 = 60_000;
/// Failed proofs in a row on one pairing before the host ends it (docs/SHARING.md §10).
pub const MAX_FAILED_PROOFS: u32 = 3;
/// Largest handshake frame accepted.
pub const MAX_HANDSHAKE_BYTES: usize = 16 << 10;

pub fn encode(h: &PeerHandshake) -> WireFrame {
    WireFrame::Text(serde_json::to_string(h).expect("handshake serializes"))
}

pub fn decode(frame: &WireFrame) -> Result<PeerHandshake, String> {
    match frame {
        WireFrame::Text(t) if t.len() <= MAX_HANDSHAKE_BYTES => {
            serde_json::from_str(t).map_err(|e| format!("bad handshake frame: {e}"))
        }
        WireFrame::Text(_) => Err("handshake frame too large".into()),
        WireFrame::Binary(_) => Err("binary frame during the handshake".into()),
    }
}

/// The joiner's first frame.
pub fn hello(
    credential: Credential,
    key: &LinkKey,
    fp_joiner: &str,
    fp_host: &str,
    name: &str,
    color: Option<ether_protocol::model::Color>,
    app: &str,
) -> PeerHandshake {
    PeerHandshake::Hello {
        share_protocol: SHARE_PROTOCOL_VERSION,
        collab_protocol: COLLAB_PROTOCOL_VERSION,
        credential,
        proof: keys::join_proof(key, fp_joiner, fp_host),
        name: name.to_string(),
        color,
        app: app.to_string(),
    }
}

/// The keys a host accepts.
#[derive(Default)]
pub struct HostKeys<'a> {
    pub edit: Option<&'a LinkKey>,
    pub listen: Option<&'a LinkKey>,
    /// `(member, key, role)`.
    pub members: Vec<(&'a str, &'a LinkKey, ShareRole)>,
}

/// Who a `Hello` proved to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admitted {
    /// A link key: a new member (the host issues a member id and key in `Welcome`).
    Link { role: ShareRole, key: LinkKey },
    /// A member key (rejoin).
    Member {
        member: MemberId,
        role: ShareRole,
        key: LinkKey,
    },
}

impl Admitted {
    pub fn role(&self) -> ShareRole {
        match self {
            Self::Link { role, .. } | Self::Member { role, .. } => *role,
        }
    }

    /// The key the `Welcome` proof is made with.
    pub fn key(&self) -> &LinkKey {
        match self {
            Self::Link { key, .. } | Self::Member { key, .. } => key,
        }
    }
}

/// Why a `Hello` was refused (sent as `PeerHandshake::Refused`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reason: JoinFailure,
    pub message: String,
}

impl Refusal {
    pub fn frame(&self) -> PeerHandshake {
        PeerHandshake::Refused {
            reason: self.reason,
            message: self.message.clone(),
        }
    }
}

fn refusal(reason: JoinFailure, message: &str) -> Refusal {
    Refusal {
        reason,
        message: message.into(),
    }
}

/// Host: check a joiner's `Hello` against `keys`. Link keys are all tried (no early exit),
/// each comparison in constant time.
pub fn admit(
    hello: &PeerHandshake,
    keys: &HostKeys<'_>,
    fp_joiner: &str,
    fp_host: &str,
) -> Result<Admitted, Refusal> {
    let PeerHandshake::Hello {
        share_protocol,
        collab_protocol,
        credential,
        proof,
        ..
    } = hello
    else {
        return Err(refusal(JoinFailure::Refused, "expected a hello"));
    };
    if *share_protocol != SHARE_PROTOCOL_VERSION || *collab_protocol != COLLAB_PROTOCOL_VERSION {
        return Err(refusal(
            JoinFailure::Version,
            "different versions of Ethereal: update both apps",
        ));
    }
    let invalid = || refusal(JoinFailure::InvalidInvite, "invalid or reset invite");
    match credential {
        Credential::Link => {
            let ok = |k: Option<&LinkKey>| {
                k.is_some_and(|k| keys::verify_join_proof(k, fp_joiner, fp_host, proof))
            };
            let edit = ok(keys.edit);
            let listen = ok(keys.listen);
            match (edit, listen) {
                (true, _) => Ok(Admitted::Link {
                    role: ShareRole::Edit,
                    key: keys.edit.expect("verified").clone(),
                }),
                (false, true) => Ok(Admitted::Link {
                    role: ShareRole::Listen,
                    key: keys.listen.expect("verified").clone(),
                }),
                _ => Err(invalid()),
            }
        }
        Credential::Member { member } => keys
            .members
            .iter()
            .find(|(m, _, _)| m == member)
            .filter(|(_, k, _)| keys::verify_join_proof(k, fp_joiner, fp_host, proof))
            .map(|(m, k, role)| Admitted::Member {
                member: (*m).to_string(),
                role: *role,
                key: (*k).clone(),
            })
            .ok_or_else(invalid),
    }
}

/// Joiner: is this `Welcome` from someone holding `key` on this very connection?
pub fn verify_welcome(
    welcome: &PeerHandshake,
    key: &LinkKey,
    fp_host: &str,
    fp_joiner: &str,
) -> bool {
    match welcome {
        PeerHandshake::Welcome { host_proof, .. } => {
            keys::verify_host_proof(key, fp_host, fp_joiner, host_proof)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::model::{Color, ProjectId};
    use ether_protocol::share::ParticipantSummary;

    const FJ: &str = "sha-256 0A:0B";
    const FH: &str = "sha-256 1A:1B";

    fn welcome(key: &LinkKey, fp_host: &str, fp_joiner: &str) -> PeerHandshake {
        PeerHandshake::Welcome {
            host_proof: keys::host_proof(key, fp_host, fp_joiner),
            role: ShareRole::Edit,
            member: "m".into(),
            member_key: None,
            host: ParticipantSummary {
                name: "Diego".into(),
                color: Color(1),
            },
            project: ProjectId::NIL,
            project_name: "Song".into(),
            online: vec![],
        }
    }

    #[test]
    fn link_keys_give_their_role() {
        let (e, l) = (keys::new_key().unwrap(), keys::new_key().unwrap());
        let hk = HostKeys {
            edit: Some(&e),
            listen: Some(&l),
            members: vec![],
        };
        let h = hello(Credential::Link, &l, FJ, FH, "Ada", None, "test");
        let a = admit(&h, &hk, FJ, FH).unwrap();
        assert_eq!(a.role(), ShareRole::Listen);
        let h = hello(Credential::Link, &e, FJ, FH, "Ada", None, "test");
        assert_eq!(admit(&h, &hk, FJ, FH).unwrap().role(), ShareRole::Edit);
        // A reset (or turned off) link no longer admits.
        let hk = HostKeys {
            edit: None,
            listen: Some(&l),
            members: vec![],
        };
        assert_eq!(
            admit(&h, &hk, FJ, FH).unwrap_err().reason,
            JoinFailure::InvalidInvite
        );
    }

    #[test]
    fn proofs_are_bound_to_the_connection() {
        let e = keys::new_key().unwrap();
        let hk = HostKeys {
            edit: Some(&e),
            ..HostKeys::default()
        };
        // A man in the middle (other fingerprints on one side) cannot relay the proof.
        let h = hello(
            Credential::Link,
            &e,
            FJ,
            "sha-256 EV:IL",
            "Ada",
            None,
            "test",
        );
        assert!(admit(&h, &hk, FJ, FH).is_err());
        assert!(verify_welcome(&welcome(&e, FH, FJ), &e, FH, FJ));
        assert!(!verify_welcome(
            &welcome(&e, FH, FJ),
            &e,
            "sha-256 EV:IL",
            FJ
        ));
        let other = keys::new_key().unwrap();
        assert!(!verify_welcome(&welcome(&other, FH, FJ), &e, FH, FJ));
    }

    #[test]
    fn member_keys_and_versions() {
        let m = keys::new_key().unwrap();
        let hk = HostKeys {
            members: vec![("mem", &m, ShareRole::Listen)],
            ..HostKeys::default()
        };
        let cred = Credential::Member {
            member: "mem".into(),
        };
        let h = hello(cred.clone(), &m, FJ, FH, "Ada", None, "test");
        assert_eq!(
            admit(&h, &hk, FJ, FH),
            Ok(Admitted::Member {
                member: "mem".into(),
                role: ShareRole::Listen,
                key: m.clone()
            })
        );
        let other = Credential::Member {
            member: "other".into(),
        };
        let h2 = hello(other, &m, FJ, FH, "Ada", None, "test");
        assert!(admit(&h2, &hk, FJ, FH).is_err());
        let PeerHandshake::Hello {
            credential,
            proof,
            name,
            color,
            app,
            ..
        } = h
        else {
            unreachable!()
        };
        let old = PeerHandshake::Hello {
            share_protocol: 0,
            collab_protocol: COLLAB_PROTOCOL_VERSION,
            credential,
            proof,
            name,
            color,
            app,
        };
        assert_eq!(
            admit(&old, &hk, FJ, FH).unwrap_err().reason,
            JoinFailure::Version
        );
    }

    #[test]
    fn frames_roundtrip() {
        let f = encode(&PeerHandshake::Accept);
        assert_eq!(decode(&f), Ok(PeerHandshake::Accept));
        assert!(decode(&WireFrame::Binary(vec![1])).is_err());
        assert!(decode(&WireFrame::Text("x".repeat(MAX_HANDSHAKE_BYTES + 1))).is_err());
    }
}
