//! `share.json`: the host-local sharing state stored next to a project's `project.ether`
//! (docs/SHARING.md §6.4). Frozen shape (versioned like `.ether`).
//!
//! It holds secrets (link keys, member keys, the host token), so it is **never** part of the
//! replicated document, a snapshot, a media push, an export, or a copy: `SaveAs` and
//! `Duplicate` must not copy it (a duplicated project is a new, unshared project), and
//! deleting the project deletes it.

use std::collections::BTreeMap;

use ether_protocol::model::{Color, SiteId};
use ether_protocol::share::{MemberId, ParticipantSummary, RoomId, ShareRole};
use serde::{Deserialize, Serialize};

/// File name, next to `project.ether`.
pub const SHARE_FILE: &str = "share.json";
/// Current `version`.
pub const SHARE_FILE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ShareFile {
    /// This store holds the master copy and shares it.
    Host(HostShare),
    /// An offline copy of someone else's shared project.
    Copy(CopyShare),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostShare {
    pub version: u32,
    pub room: RoomId,
    /// `None` = the default service.
    pub signal_url: Option<String>,
    /// Proves the room claim to the signaling service (32 random bytes, base64url).
    pub host_token: String,
    /// Current link keys (`LinkKey` text: version + secret). `None`: that link is off.
    pub edit_key: Option<String>,
    pub listen_key: Option<String>,
    pub members: Vec<MemberRecord>,
    /// Last sequenced `seq` per site, kept across hub restarts so a returning site's
    /// resends are recognized (it seeds the hub's first `SnapshotData::sites`).
    #[serde(default)]
    pub sites: BTreeMap<SiteId, u64>,
    /// Sharing was on when the project was last closed: opening it resumes sharing.
    pub resume: bool,
}

/// Someone who joined once and may come back (survives link resets; removed with
/// `RemoveParticipant`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub member: MemberId,
    /// Member key (`LinkKey` text).
    pub key: String,
    pub role: ShareRole,
    pub name: String,
    pub color: Color,
    pub last_seen_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopyShare {
    pub version: u32,
    pub room: RoomId,
    pub signal_url: Option<String>,
    pub member: MemberId,
    /// Member key (`LinkKey` text) issued by the host.
    pub key: String,
    pub role: ShareRole,
    pub host_name: String,
    /// Last known participants (host first, at most 8).
    pub participants: Vec<ParticipantSummary>,
    pub last_synced_ms: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_is_tagged_by_kind() {
        let f = ShareFile::Copy(CopyShare {
            version: SHARE_FILE_VERSION,
            room: "r".into(),
            signal_url: None,
            member: "m".into(),
            key: "1k".into(),
            role: ShareRole::Listen,
            host_name: "Diego".into(),
            participants: vec![],
            last_synced_ms: Some(1.0),
        });
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["kind"], "Copy");
        assert_eq!(serde_json::from_value::<ShareFile>(json).unwrap(), f);

        let mut sites = BTreeMap::new();
        sites.insert(SiteId(42), 7);
        let h = ShareFile::Host(HostShare {
            version: SHARE_FILE_VERSION,
            room: "r".into(),
            signal_url: None,
            host_token: "t".into(),
            edit_key: Some("1e".into()),
            listen_key: None,
            members: vec![],
            sites,
            resume: true,
        });
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains("\"sites\":{\"42\":7}"), "{json}");
        assert_eq!(serde_json::from_str::<ShareFile>(&json).unwrap(), h);
    }
}
