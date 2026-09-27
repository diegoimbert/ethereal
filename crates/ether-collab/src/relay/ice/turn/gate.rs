//! What reaches the TURN crate, and when its nonce map must be replaced (pure, injected
//! time; docs/COLLAB.md §10).
//!
//! The crate inserts a nonce into a map it never evicts each time it answers a request with
//! a 401 (no MESSAGE-INTEGRITY) or a 438 (a NONCE it does not know, or one older than an
//! hour, which it removes first). [`NonceLedger`] mirrors that map from the responses we
//! see leaving the socket, so the budgets count real inserts (and shrink when the crate
//! removes a stale nonce). [`RequestGate`] predicts, per request, whether handing it to the
//! crate costs an insert: requests that do not (a valid-looking authenticated request with
//! a nonce the crate still holds) only face the rate caps; past the soft budget, requests
//! that do are let through only under per-username and global caps (MESSAGE-INTEGRITY
//! verified by us) or a small per-source and global trickle (unverified: a new client's
//! first Allocate). [`rotation`] decides when the server is replaced.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use super::super::stun::RateCaps;

/// Nonces the crate holds before only requests that cost no insert, or are capped (see the
/// module docs), are handed to it; with no live allocation, the server is rotated.
pub const NONCE_SOFT_BUDGET: u64 = 50_000;
/// Nonces the crate holds before the server is rotated even with live allocations.
pub const NONCE_HARD_BUDGET: u64 = 100_000;
/// Unverified requests that cost an insert, let through per second (whole relay) past the
/// soft budget.
pub const UNVERIFIED_TRICKLE_PER_SECOND: f64 = 5.0;
/// Of that trickle, one source IP gets this many per second (burst
/// [`UNVERIFIED_PER_SOURCE_BURST`]), so one sender cannot take all of it.
pub const UNVERIFIED_PER_SOURCE_PER_SECOND: f64 = 0.5;
/// A client's first Allocate of each of its allocations is unverified.
pub const UNVERIFIED_PER_SOURCE_BURST: f64 = 4.0;
/// Verified requests that cost an insert (a 438: the nonce is stale or unknown, e.g. after
/// a rotation), per username per second past the soft budget (burst
/// [`VERIFIED_INSERTS_PER_USERNAME_BURST`]). A legitimate client needs about one per
/// allocation per hour; a replayed captured request is held to this rate.
pub const VERIFIED_INSERTS_PER_USERNAME_PER_SECOND: f64 = 0.1;
pub const VERIFIED_INSERTS_PER_USERNAME_BURST: f64 = 8.0;
/// Verified requests that cost an insert, per second for the whole relay past the soft
/// budget (bounds many usernames together).
pub const VERIFIED_INSERTS_PER_SECOND: f64 = 20.0;
/// Source IPs / usernames tracked by the per-key caps; past it, idle keys are forgotten
/// and new keys are refused while none is idle.
pub const MAX_TRACKED_KEYS: usize = 4096;
/// The crate's nonce lifetime (`turn::server::request::NONCE_LIFETIME`, crate-private).
pub const NONCE_LIFETIME: Duration = Duration::from_secs(3600);

/// The gate's limits (defaults: the constants above). Tests shrink them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnBudgets {
    pub nonce_soft: u64,
    pub nonce_hard: u64,
    pub unverified_per_second: f64,
    pub unverified_per_source_per_second: f64,
    pub unverified_per_source_burst: f64,
    pub verified_inserts_per_username_per_second: f64,
    pub verified_inserts_per_username_burst: f64,
    pub verified_inserts_per_second: f64,
    /// Per source IP and global request caps (every request, before the budgets).
    pub requests_per_ip_per_second: u32,
    pub requests_per_second: u32,
    pub max_tracked_keys: usize,
}

impl Default for TurnBudgets {
    fn default() -> Self {
        use super::super::stun::{GLOBAL_PER_SECOND, PER_IP_PER_SECOND};
        Self {
            nonce_soft: NONCE_SOFT_BUDGET,
            nonce_hard: NONCE_HARD_BUDGET,
            unverified_per_second: UNVERIFIED_TRICKLE_PER_SECOND,
            unverified_per_source_per_second: UNVERIFIED_PER_SOURCE_PER_SECOND,
            unverified_per_source_burst: UNVERIFIED_PER_SOURCE_BURST,
            verified_inserts_per_username_per_second: VERIFIED_INSERTS_PER_USERNAME_PER_SECOND,
            verified_inserts_per_username_burst: VERIFIED_INSERTS_PER_USERNAME_BURST,
            verified_inserts_per_second: VERIFIED_INSERTS_PER_SECOND,
            requests_per_ip_per_second: PER_IP_PER_SECOND,
            requests_per_second: GLOBAL_PER_SECOND,
            max_tracked_keys: MAX_TRACKED_KEYS,
        }
    }
}

/// Why the server must be replaced (a fresh one has an empty nonce map).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// Past the soft budget with no live allocation: nobody notices.
    Idle,
    /// At the hard budget: live allocations are dropped.
    Forced,
}

/// Whether to rotate the server holding `nonces` nonces with `live` live allocations.
pub fn rotation(nonces: u64, live: usize, budgets: &TurnBudgets) -> Option<Rotation> {
    if nonces >= budgets.nonce_hard {
        Some(Rotation::Forced)
    } else if nonces >= budgets.nonce_soft && live == 0 {
        Some(Rotation::Idle)
    } else {
        None
    }
}

/// A token bucket (`rate` per second, at most `burst`), starting full.
#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(burst: f64, now: Instant) -> Self {
        Self {
            tokens: burst,
            at: now,
        }
    }

    fn refill(&mut self, rate: f64, burst: f64, now: Instant) {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + dt * rate).min(burst);
        self.at = self.at.max(now);
    }
}

/// A token bucket per key, at most `max_keys` of them (full buckets are forgotten first:
/// they are indistinguishable from a new key).
#[derive(Debug)]
struct KeyedBuckets<K> {
    buckets: HashMap<K, Bucket>,
    rate: f64,
    burst: f64,
    max_keys: usize,
}

impl<K: Hash + Eq> KeyedBuckets<K> {
    fn new(rate: f64, burst: f64, max_keys: usize) -> Self {
        Self {
            buckets: HashMap::new(),
            rate,
            burst,
            max_keys: max_keys.max(1),
        }
    }

    /// Whether `key` has a token at `now` (refilled, not taken).
    fn has(&mut self, key: K, now: Instant) -> Option<&mut Bucket> {
        let (rate, burst) = (self.rate, self.burst);
        if !self.buckets.contains_key(&key) && self.buckets.len() >= self.max_keys {
            self.buckets.retain(|_, b| {
                b.refill(rate, burst, now);
                b.tokens < burst
            });
            if self.buckets.len() >= self.max_keys {
                return None;
            }
        }
        let b = self
            .buckets
            .entry(key)
            .or_insert_with(|| Bucket::full(burst, now));
        b.refill(rate, burst, now);
        (b.tokens >= 1.0).then_some(b)
    }
}

/// A mirror of the crate's nonce map (hashes of the nonces it handed out, with when).
#[derive(Debug, Default)]
pub struct NonceLedger {
    issued: HashMap<u64, Instant>,
    hasher: RandomState,
}

/// What the crate will do with a request's NONCE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonceState {
    /// Held and younger than [`NONCE_LIFETIME`]: accepted, no insert.
    Fresh,
    /// Unknown, or held but expired (the crate removes it): answered with a 438 (insert).
    Stale,
}

impl NonceLedger {
    fn key(&self, nonce: &[u8]) -> u64 {
        self.hasher.hash_one(nonce)
    }

    /// The crate handed out `nonce` (a 401/438 response left the socket).
    pub fn issued(&mut self, nonce: &[u8], now: Instant) {
        let key = self.key(nonce);
        self.issued.insert(key, now);
    }

    pub fn state(&self, nonce: &[u8], now: Instant) -> NonceState {
        match self.issued.get(&self.key(nonce)) {
            Some(at) if now.saturating_duration_since(*at) < NONCE_LIFETIME => NonceState::Fresh,
            _ => NonceState::Stale,
        }
    }

    /// `nonce` was handed to the crate in a request: if it was expired the crate drops it.
    pub fn presented(&mut self, nonce: &[u8], now: Instant) {
        let key = self.key(nonce);
        if let Some(at) = self.issued.get(&key)
            && now.saturating_duration_since(*at) >= NONCE_LIFETIME
        {
            self.issued.remove(&key);
        }
    }

    /// Nonces the crate holds (as far as we saw).
    pub fn len(&self) -> u64 {
        self.issued.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.issued.is_empty()
    }

    pub fn clear(&mut self) {
        self.issued.clear();
    }
}

/// What the gate needs to know about a request (parsed by the caller; see [`inspect`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestInfo {
    /// Parsed as STUN with a MESSAGE-INTEGRITY attribute (not yet checked).
    pub has_integrity: bool,
    /// The NONCE attribute (`Some(None)`: present but not valid UTF-8 text).
    pub nonce: Option<Option<Vec<u8>>>,
}

/// Parse `packet` for the gate (`None` if it is not a well-formed STUN message: the crate
/// drops it without an answer, but it is still counted as costing an insert).
pub fn inspect(packet: &[u8]) -> Option<RequestInfo> {
    let mut m = stun::message::Message::new();
    m.unmarshal_binary(packet).ok()?;
    let nonce = m
        .get(stun::attributes::ATTR_NONCE)
        .ok()
        .map(|n| std::str::from_utf8(&n).is_ok().then_some(n));
    Some(RequestInfo {
        has_integrity: m.contains(stun::attributes::ATTR_MESSAGE_INTEGRITY),
        nonce,
    })
}

/// The NONCE of a STUN error response (what the crate sends with a 401/438), if any.
pub fn issued_nonce(packet: &[u8]) -> Option<Vec<u8>> {
    if packet.len() < 2 || packet[0] & 0xc0 != 0 {
        return None;
    }
    let typ = u16::from_be_bytes([packet[0], packet[1]]);
    if typ & 0x0110 != 0x0110 {
        return None;
    }
    let mut m = stun::message::Message::new();
    m.unmarshal_binary(packet).ok()?;
    m.get(stun::attributes::ATTR_NONCE).ok()
}

/// What reaches the crate from the listening socket (Binding requests excluded): every
/// STUN request is rate-capped per source IP and globally; past the soft budget, requests
/// that cost a nonce insert are capped (see the module docs). Pure, for tests.
#[derive(Debug)]
pub struct RequestGate {
    budgets: TurnBudgets,
    caps: RateCaps,
    ledger: NonceLedger,
    unverified: Bucket,
    unverified_per_source: KeyedBuckets<IpAddr>,
    verified: Bucket,
    verified_per_username: KeyedBuckets<String>,
}

impl Default for RequestGate {
    fn default() -> Self {
        Self::new(TurnBudgets::default(), Instant::now())
    }
}

impl RequestGate {
    pub fn new(budgets: TurnBudgets, now: Instant) -> Self {
        let b = budgets;
        Self {
            budgets,
            caps: RateCaps::new(
                b.requests_per_ip_per_second,
                b.requests_per_second,
                b.max_tracked_keys,
            ),
            ledger: NonceLedger::default(),
            unverified: Bucket::full(b.unverified_per_second, now),
            unverified_per_source: KeyedBuckets::new(
                b.unverified_per_source_per_second,
                b.unverified_per_source_burst,
                b.max_tracked_keys,
            ),
            verified: Bucket::full(b.verified_inserts_per_second, now),
            verified_per_username: KeyedBuckets::new(
                b.verified_inserts_per_username_per_second,
                b.verified_inserts_per_username_burst,
                b.max_tracked_keys,
            ),
        }
    }

    pub fn budgets(&self) -> &TurnBudgets {
        &self.budgets
    }

    /// `packet` from `from`: `verify` returns the username whose current TURN REST
    /// credential its MESSAGE-INTEGRITY checks against (only called when it matters).
    /// ChannelData, indications and responses get no answer (they reflect nothing, and
    /// nonces only come from requests): always passed.
    pub fn admit(
        &mut self,
        packet: &[u8],
        from: IpAddr,
        now: Instant,
        verify: impl FnOnce() -> Option<String>,
    ) -> bool {
        if !is_stun_request(packet) {
            return true;
        }
        if !self.caps.admit(from, now) {
            return false;
        }
        let info = inspect(packet);
        let inserts = match &info {
            // No MESSAGE-INTEGRITY: a 401. Malformed: dropped by the crate, but counted.
            None => true,
            Some(RequestInfo {
                has_integrity: false,
                ..
            }) => true,
            // Authenticated without a NONCE: a 400, nothing stored.
            Some(RequestInfo { nonce: None, .. }) => false,
            Some(RequestInfo {
                nonce: Some(None), ..
            }) => true,
            Some(RequestInfo {
                nonce: Some(Some(n)),
                ..
            }) => self.ledger.state(n, now) == NonceState::Stale,
        };
        let pass = !inserts
            || self.ledger.len() < self.budgets.nonce_soft
            || match verify() {
                Some(username) => self.take_verified(username, now),
                None => self.take_unverified(from, now),
            };
        if pass
            && let Some(RequestInfo {
                nonce: Some(Some(n)),
                ..
            }) = &info
        {
            self.ledger.presented(n, now);
        }
        pass
    }

    fn take_verified(&mut self, username: String, now: Instant) -> bool {
        let b = &self.budgets;
        self.verified.refill(
            b.verified_inserts_per_second,
            b.verified_inserts_per_second,
            now,
        );
        if self.verified.tokens < 1.0 {
            return false;
        }
        let Some(user) = self.verified_per_username.has(username, now) else {
            return false;
        };
        user.tokens -= 1.0;
        self.verified.tokens -= 1.0;
        true
    }

    /// Past the soft budget, unverified requests (a new client's first Allocate) still get
    /// through at the trickle rate for the whole relay, a few per source, so new clients
    /// can join while the server waits to rotate.
    fn take_unverified(&mut self, from: IpAddr, now: Instant) -> bool {
        let rate = self.budgets.unverified_per_second;
        self.unverified.refill(rate, rate, now);
        if self.unverified.tokens < 1.0 {
            return false;
        }
        let Some(source) = self.unverified_per_source.has(from.to_canonical(), now) else {
            return false;
        };
        source.tokens -= 1.0;
        self.unverified.tokens -= 1.0;
        true
    }

    /// The crate sent `packet` (see [`issued_nonce`]).
    pub fn sent(&mut self, packet: &[u8], now: Instant) {
        if let Some(nonce) = issued_nonce(packet) {
            self.ledger.issued(&nonce, now);
        }
    }

    /// Nonces the current server holds (as far as we saw).
    pub fn nonces(&self) -> u64 {
        self.ledger.len()
    }

    /// A fresh server (and nonce map) takes over.
    pub fn rotated(&mut self) {
        self.ledger.clear();
    }
}

/// A STUN request (not ChannelData, not an indication or response). Short datagrams
/// that could be a truncated request count as one (dropped by the crate, but capped).
fn is_stun_request(packet: &[u8]) -> bool {
    if packet.len() < 2 {
        return true;
    }
    let typ = u16::from_be_bytes([packet[0], packet[1]]);
    typ & 0xc000 == 0 && typ & 0x0110 == 0
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use stun::attributes::{ATTR_NONCE, ATTR_REALM, ATTR_USERNAME};
    use stun::message::{
        CLASS_ERROR_RESPONSE, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE, METHOD_ALLOCATE, Message,
        MessageClass, MessageType, Setter,
    };
    use stun::textattrs::TextAttribute;

    use super::*;

    fn message(class: MessageClass, attrs: Vec<Box<dyn Setter>>) -> Vec<u8> {
        let mut setters: Vec<Box<dyn Setter>> = vec![
            Box::new(stun::agent::TransactionId::new()),
            Box::new(MessageType::new(METHOD_ALLOCATE, class)),
        ];
        setters.extend(attrs);
        let mut m = Message::new();
        m.build(&setters).unwrap();
        m.raw
    }

    /// A first Allocate (no MESSAGE-INTEGRITY): the crate answers 401 with a nonce.
    fn first_allocate() -> Vec<u8> {
        message(CLASS_REQUEST, vec![])
    }

    /// An authenticated Allocate presenting `nonce` (its integrity is judged by `verify`).
    fn authenticated(nonce: &str, username: &str) -> Vec<u8> {
        message(
            CLASS_REQUEST,
            vec![
                Box::new(TextAttribute::new(ATTR_USERNAME, username.into())),
                Box::new(TextAttribute::new(ATTR_REALM, "ether-collab".into())),
                Box::new(TextAttribute::new(ATTR_NONCE, nonce.into())),
                Box::new(stun::integrity::MessageIntegrity::new_long_term_integrity(
                    username.into(),
                    "ether-collab".into(),
                    "pw".into(),
                )),
            ],
        )
    }

    /// The crate's 401/438 carrying `nonce`.
    fn nonce_response(nonce: &str) -> Vec<u8> {
        message(
            CLASS_ERROR_RESPONSE,
            vec![Box::new(TextAttribute::new(ATTR_NONCE, nonce.into()))],
        )
    }

    fn ip(i: u32) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(0xc633_6400u32.wrapping_add(i)))
    }

    /// Budgets without request caps (so only the nonce budgets decide).
    fn budgets(soft: u64, hard: u64) -> TurnBudgets {
        TurnBudgets {
            nonce_soft: soft,
            nonce_hard: hard,
            requests_per_ip_per_second: u32::MAX,
            requests_per_second: u32::MAX,
            ..TurnBudgets::default()
        }
    }

    /// Fill the ledger with `n` nonces, as the crate would after `n` first Allocates.
    fn fill(g: &mut RequestGate, n: u64, now: Instant) {
        for i in 0..n {
            assert!(g.admit(&first_allocate(), ip(0), now, || None));
            g.sent(&nonce_response(&format!("fill-{i}")), now);
        }
    }

    fn never() -> Option<String> {
        panic!("verification not needed here")
    }

    #[test]
    fn every_request_is_capped_even_with_a_valid_integrity() {
        use super::super::super::stun::{GLOBAL_PER_SECOND, PER_IP_PER_SECOND};
        let now = Instant::now();
        let mut g = RequestGate::new(TurnBudgets::default(), now);
        let p = authenticated("n", "u");
        let valid = || Some("u".to_string());
        let passed = (0..200).filter(|_| g.admit(&p, ip(0), now, valid)).count();
        assert_eq!(passed, PER_IP_PER_SECOND as usize);
        // Other sources share the global cap.
        let mut total = passed;
        for i in 1..=100 {
            total += (0..60).filter(|_| g.admit(&p, ip(i), now, valid)).count();
        }
        assert_eq!(total, GLOBAL_PER_SECOND as usize);
        // ChannelData and indications are not requests: never capped here.
        let channel_data = [0x40, 0x00, 0x00, 0x04, 1, 2, 3, 4];
        let indication = [0x00, 0x16, 0x00, 0x00];
        assert!(g.admit(&channel_data, ip(0), now, never));
        assert!(g.admit(&indication, ip(0), now, never));
    }

    #[test]
    fn the_budget_counts_nonces_the_crate_sent_not_admitted_requests() {
        let now = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), now);
        for _ in 0..100 {
            assert!(g.admit(&first_allocate(), ip(1), now, || None));
        }
        assert_eq!(g.nonces(), 0, "nothing answered yet");
        g.sent(&nonce_response("a"), now);
        g.sent(&nonce_response("b"), now);
        // A success response, ChannelData and a Data indication carry no nonce.
        g.sent(&message(CLASS_SUCCESS_RESPONSE, vec![]), now);
        g.sent(&[0x40, 0x00, 0x00, 0x04, 1, 2, 3, 4], now);
        g.sent(&[0x00, 0x17, 0x00, 0x00], now);
        assert_eq!(g.nonces(), 2);
        // Authenticated requests with a nonce the crate holds add nothing, however many.
        for _ in 0..1000 {
            assert!(g.admit(&authenticated("a", "u"), ip(1), now, never));
        }
        assert_eq!(g.nonces(), 2);
        g.rotated();
        assert_eq!(g.nonces(), 0);
    }

    #[test]
    fn the_ledger_forgets_a_nonce_the_crate_drops_as_stale() {
        let t0 = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), t0);
        g.sent(&nonce_response("old"), t0);
        g.sent(&nonce_response("new"), t0 + Duration::from_secs(1800));
        let t1 = t0 + NONCE_LIFETIME;
        assert_eq!(g.ledger.state(b"new", t1), NonceState::Fresh);
        assert_eq!(g.ledger.state(b"old", t1), NonceState::Stale);
        assert_eq!(g.ledger.state(b"unknown", t1), NonceState::Stale);
        // Presenting the fresh one keeps it; the expired one is removed (the crate removes
        // it and answers 438 with a new one, which is then counted).
        assert!(g.admit(&authenticated("new", "u"), ip(1), t1, never));
        assert!(g.admit(&authenticated("old", "u"), ip(1), t1, || None));
        assert_eq!(g.nonces(), 1);
        g.sent(&nonce_response("renewed"), t1);
        assert_eq!(g.nonces(), 2);
        // An unknown nonce removes nothing.
        assert!(g.admit(&authenticated("unknown", "u"), ip(1), t1, || None));
        assert_eq!(g.nonces(), 2);
    }

    #[test]
    fn past_the_soft_budget_requests_that_cost_no_insert_still_pass() {
        let now = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), now);
        fill(&mut g, 10, now);
        // A nonce the crate holds: no insert, no verification needed.
        for _ in 0..100 {
            assert!(g.admit(&authenticated("fill-3", "u"), ip(2), now, never));
        }
        assert_eq!(g.nonces(), 10);
    }

    #[test]
    fn past_the_soft_budget_verified_inserts_are_capped_per_username() {
        let t0 = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), t0);
        fill(&mut g, 10, t0);
        // A captured request (stale nonce, valid integrity) replayed from spoofed sources.
        let replay = authenticated("stale", "1700000000:7");
        let user = |u: &'static str| move || Some(u.to_string());
        let burst = VERIFIED_INSERTS_PER_USERNAME_BURST as usize;
        let passed = (0..100)
            .filter(|&i| g.admit(&replay, ip(i), t0, user("1700000000:7")))
            .count();
        assert_eq!(passed, burst, "spoofed sources do not multiply the cap");
        // Another username is not held back by that one.
        assert!(g.admit(&replay, ip(0), t0, user("1700000000:8")));
        // Refill: one per 10 s.
        let t1 = t0 + Duration::from_secs(10);
        let passed = (0..100)
            .filter(|&i| g.admit(&replay, ip(i), t1, user("1700000000:7")))
            .count();
        assert_eq!(passed, 1);
    }

    #[test]
    fn past_the_soft_budget_verified_inserts_are_capped_globally() {
        let now = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), now);
        fill(&mut g, 10, now);
        let p = authenticated("stale", "x");
        let passed = (0..1000)
            .filter(|&i| g.admit(&p, ip(i), now, move || Some(format!("user-{i}"))))
            .count();
        assert_eq!(passed, VERIFIED_INSERTS_PER_SECOND as usize);
    }

    #[test]
    fn past_the_soft_budget_unverified_requests_trickle_a_few_per_source() {
        let t0 = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), t0);
        fill(&mut g, 10, t0);
        let p = first_allocate();
        // One sender gets its per-source burst, not the whole trickle...
        let per_source = UNVERIFIED_PER_SOURCE_BURST as usize;
        let passed = (0..50).filter(|_| g.admit(&p, ip(1), t0, || None)).count();
        assert_eq!(passed, per_source);
        // ...so a new client still gets through, until the global trickle is spent.
        let trickle = UNVERIFIED_TRICKLE_PER_SECOND as usize;
        let passed = (2..50).filter(|&i| g.admit(&p, ip(i), t0, || None)).count();
        assert_eq!(passed, trickle - per_source);
        // A forged integrity is unverified too.
        let t1 = t0 + Duration::from_secs(1);
        assert!(g.admit(&authenticated("stale", "x"), ip(60), t1, || None));
        // Never more than one second's worth, however long it waited.
        let t2 = t0 + Duration::from_secs(60);
        let passed = (100..200)
            .filter(|&i| g.admit(&p, ip(i), t2, || None))
            .count();
        assert_eq!(passed, trickle);
    }

    #[test]
    fn under_the_soft_budget_nothing_is_verified() {
        let now = Instant::now();
        let mut g = RequestGate::new(budgets(10, 20), now);
        fill(&mut g, 9, now);
        assert!(g.admit(&first_allocate(), ip(1), now, never));
        assert!(g.admit(&authenticated("stale", "u"), ip(1), now, never));
    }

    #[test]
    fn rotation_decisions() {
        let b = budgets(10, 20);
        assert_eq!(rotation(0, 0, &b), None);
        assert_eq!(rotation(9, 0, &b), None);
        assert_eq!(
            rotation(10, 0, &b),
            Some(Rotation::Idle),
            "soft, nobody live"
        );
        assert_eq!(rotation(10, 1, &b), None, "soft: live allocations are kept");
        assert_eq!(rotation(19, 64, &b), None);
        assert_eq!(
            rotation(20, 3, &b),
            Some(Rotation::Forced),
            "hard: live dropped"
        );
        assert_eq!(rotation(20, 0, &b), Some(Rotation::Forced));
        let d = TurnBudgets::default();
        assert_eq!(rotation(NONCE_SOFT_BUDGET - 1, 0, &d), None);
        assert_eq!(rotation(NONCE_SOFT_BUDGET, 0, &d), Some(Rotation::Idle));
        assert_eq!(rotation(NONCE_HARD_BUDGET, 1, &d), Some(Rotation::Forced));
    }

    /// Default budgets and caps, live allocations, one simulated hour: the whole global
    /// request cap spent on a captured valid request (stale nonce) replayed from spoofed
    /// sources and on unverified first Allocates from ever-new sources, each admitted one
    /// answered with a new nonce. The old gate (counting admitted requests) forced a
    /// rotation after ~100 s; past the soft budget this one grows by at most the trickle
    /// and the capped verified inserts (~5.1/s), so the hour ends far from the hard budget.
    #[test]
    fn a_replay_flood_does_not_force_a_rotation_within_an_hour() {
        let t0 = Instant::now();
        let mut g = RequestGate::new(TurnBudgets::default(), t0);
        let replay = authenticated("captured-stale-nonce", "1700000000:7");
        let flood = first_allocate();
        let mut issued = 0u64;
        for second in 0..3600u32 {
            let now = t0 + Duration::from_secs(second.into());
            for i in 0..super::super::super::stun::GLOBAL_PER_SECOND {
                let src = ip(second.wrapping_mul(1000).wrapping_add(i));
                let admitted = if i % 2 == 0 {
                    g.admit(&replay, src, now, || Some("1700000000:7".into()))
                } else {
                    g.admit(&flood, src, now, || None)
                };
                if admitted {
                    issued += 1;
                    g.sent(&nonce_response(&format!("n{issued}")), now);
                }
            }
            assert_eq!(rotation(g.nonces(), 1, g.budgets()), None, "at {second} s");
        }
        let past_soft = g.nonces() - NONCE_SOFT_BUDGET;
        assert!(
            past_soft < 3600 * 6,
            "{past_soft} nonces past the soft budget"
        );
    }
}
