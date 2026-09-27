//! Per-site throttles for high-rate messages (docs/COLLAB.md §7, §8.5). Excess messages
//! are dropped silently (never a disconnect: that is the global message rate limit's job).

use ether_protocol::collab::CollabMessage;

/// At most one `Presence` per site per this interval (20 Hz; sites send ≤ 10 Hz).
pub const PRESENCE_INTERVAL_MS: u64 = 50;
/// At most one `Pointer` per site per this interval (40 Hz; sites send ≤ 30 Hz). A clear
/// (`pointer: None`) always passes, so a pointer never sticks.
pub const POINTER_INTERVAL_MS: u64 = 25;
/// Site-to-site messages (signaling, listen, transport requests, stream clocks): token
/// bucket of this many per second, burst of one second's worth. A host with 8 listeners
/// sends ~80 clock anchors/s; a negotiation is a few dozen messages.
pub const ROUTED_PER_SECOND: u32 = 200;

/// Throttle state of one connection. Times are milliseconds on any monotonic clock.
#[derive(Clone, Debug)]
pub struct SiteLimiter {
    last_presence: Option<u64>,
    last_pointer: Option<u64>,
    routed_budget: f64,
    routed_refilled: Option<u64>,
}

impl Default for SiteLimiter {
    fn default() -> Self {
        Self {
            last_presence: None,
            last_pointer: None,
            routed_budget: f64::from(ROUTED_PER_SECOND),
            routed_refilled: None,
        }
    }
}

impl SiteLimiter {
    /// Whether `m`, arriving at `now_ms`, is passed on to the relay (`false` = drop it).
    pub fn admit(&mut self, m: &CollabMessage, now_ms: u64) -> bool {
        let gap = |last: Option<u64>, interval: u64| {
            last.is_none_or(|t| now_ms.saturating_sub(t) >= interval)
        };
        match m {
            CollabMessage::Presence { .. } => {
                let ok = gap(self.last_presence, PRESENCE_INTERVAL_MS);
                if ok {
                    self.last_presence = Some(now_ms);
                }
                ok
            }
            CollabMessage::Pointer { pointer, .. } => {
                let ok = pointer.is_none() || gap(self.last_pointer, POINTER_INTERVAL_MS);
                if ok {
                    self.last_pointer = Some(now_ms);
                }
                ok
            }
            m if m.route().is_some() => {
                let rate = f64::from(ROUTED_PER_SECOND);
                let since = self.routed_refilled.map_or(0, |t| now_ms.saturating_sub(t));
                self.routed_budget = (self.routed_budget + since as f64 * rate / 1000.0).min(rate);
                self.routed_refilled = Some(now_ms);
                if self.routed_budget >= 1.0 {
                    self.routed_budget -= 1.0;
                    true
                } else {
                    false
                }
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use ether_protocol::collab::{ArrangerPointer, StreamSignal};
    use ether_protocol::model::{Beats, SiteId};

    use super::*;

    fn pointer(some: bool) -> CollabMessage {
        CollabMessage::Pointer {
            site: SiteId(1),
            pointer: some.then_some(ArrangerPointer {
                beats: Beats(1.0),
                track: None,
                y: 0.0,
                editor: None,
            }),
        }
    }

    #[test]
    fn pointer_is_capped_but_clears_always_pass() {
        let mut l = SiteLimiter::default();
        assert!(l.admit(&pointer(true), 1000));
        assert!(!l.admit(&pointer(true), 1010), "40 Hz cap");
        assert!(l.admit(&pointer(false), 1011), "a clear always passes");
        assert!(!l.admit(&pointer(true), 1020));
        assert!(l.admit(&pointer(true), 1040));
        // 30 Hz senders are never throttled.
        let mut l = SiteLimiter::default();
        assert!((0..30).all(|i| l.admit(&pointer(true), 5000 + i * 34)));
    }

    #[test]
    fn routed_messages_share_a_token_bucket() {
        let bye = CollabMessage::Signal {
            from: SiteId(1),
            to: SiteId(2),
            stream: 1,
            signal: StreamSignal::Bye { reason: None },
        };
        let mut l = SiteLimiter::default();
        let passed = (0..300).filter(|_| l.admit(&bye, 0)).count();
        assert_eq!(passed, ROUTED_PER_SECOND as usize, "burst of one second");
        assert!(!l.admit(&bye, 1));
        assert!(l.admit(&bye, 10), "refills at the rate");
        // Other messages are not affected.
        assert!(l.admit(&CollabMessage::Leave { site: SiteId(1) }, 10));
    }
}
