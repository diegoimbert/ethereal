//! Collaboration envelope (RESERVED). Roadmap v2, refined by the `collab` node via BCR (see
//! `docs/ROADMAP.md`).
//!
//! Only the identity/stamping shapes are frozen here so ops can be attributed once several
//! replicas edit one document; the sync algorithm (Loro/Yrs, a relay, ...) lives in
//! `ether-collab`. Nothing in v0.x produces or consumes these yet.
//!
//! - A **site** is one replica of the document (one controller: a desktop app, a server
//!   instance). Its id is a random 64-bit number, like a Loro `PeerID`, serialized as a
//!   decimal string because JS numbers lose precision above 2^53.
//! - An **actor** is the human (or agent) behind an edit. Several sites may share one actor
//!   (same user on two machines).
//! - A [`StampedTransaction`] is a committed [`Transaction`] as exchanged between sites:
//!   `origin` says who made it and orders it per site (`seq`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::op::Transaction;

/// One replica of the document. Serialized as a decimal string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[ts(type = "string")]
pub struct SiteId(pub u64);

impl Serialize for SiteId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for SiteId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map(Self).map_err(serde::de::Error::custom)
    }
}

/// The user (or agent) behind an edit: an opaque, stable string (account id, ULID, ...).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
pub struct ActorId(pub String);

/// Who made a transaction, and its position in that site's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OpOrigin {
    pub site: SiteId,
    pub actor: Option<ActorId>,
    /// Per-site sequence number (1, 2, ...): transactions of one site apply in `seq` order.
    #[ts(type = "number")]
    pub seq: u64,
}

/// A committed transaction as exchanged between sites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StampedTransaction {
    pub origin: OpOrigin,
    pub transaction: Transaction,
}
