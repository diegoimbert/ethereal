//! One listener's peer connection: a str0m `Rtc` with one send-only Opus m-line
//! (docs/COLLAB.md §9.1), its signaling, state and anchor bookkeeping. Driven by the sender
//! thread (`sender.rs`), which owns the socket.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_controller::streaming::{StreamLinkState, StreamOutput};
use ether_core::protocol::collab::{IceCandidate, StreamSignal};
use ether_core::protocol::model::SiteId;
use str0m::bwe::{Bitrate, BweKind};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid, Pt};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

/// Opus payload type we offer (the browser's usual one).
pub const OPUS_PT: u8 = 111;
/// Initial / default bitrate (docs/COLLAB.md §9.1).
pub const DEFAULT_BITRATE: u64 = 128_000;
/// What the BWE probes towards (the top of the adapted range).
pub const MAX_BITRATE: u64 = 192_000;
pub const MIN_BITRATE: u64 = 48_000;

/// A connected link whose ICE stays disconnected this long has failed (consent freshness).
pub const DISCONNECTED_FAIL: Duration = Duration::from_secs(8);
/// A link that never connects within this long (after the offer) has failed.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Peer {
    pub listener: SiteId,
    pub stream: u32,
    pub rtc: Rtc,
    pub mid: Mid,
    pending: Option<SdpPendingOffer>,
    /// This listener's random RTP offset `o`.
    pub offset: u32,
    pub connected: bool,
    created: Instant,
    disconnected_since: Option<Instant>,
    pub next_timeout: Instant,
    /// Stream index of the last anchor sent (`None`: none yet).
    pub last_anchor_n: Option<u64>,
    /// Latest egress bitrate estimate (bit/s).
    pub estimate: Option<u64>,
    /// STUN queries still outstanding for this peer (srflx gathering).
    pub stun_pending: usize,
    eoc_sent: bool,
    /// Terminal state reached (the peer is removed after the current drive).
    pub done: Option<StreamLinkState>,
}

impl Peer {
    /// A new peer connection and its offer (with `host` candidates embedded).
    pub fn new(listener: SiteId, stream: u32, host: &[SocketAddr], now: Instant) -> (Self, String) {
        let mut cfg = Rtc::builder()
            .clear_codecs()
            .enable_bwe(Some(Bitrate::bps(DEFAULT_BITRATE)));
        let fmt = FormatParams {
            min_p_time: Some(10),
            use_inband_fec: Some(true),
            stereo: Some(true),
            sprop_stereo: Some(true),
            ..FormatParams::default()
        };
        cfg.codec_config().add_config(
            Pt::from(OPUS_PT),
            None,
            Codec::Opus,
            Frequency::FORTY_EIGHT_KHZ,
            Some(2),
            fmt,
        );
        let mut rtc = cfg.build(now);
        rtc.bwe().set_desired_bitrate(Bitrate::bps(MAX_BITRATE));
        for &a in host {
            if let Ok(c) = Candidate::host(a, "udp") {
                rtc.add_local_candidate(c);
            }
        }
        let mut api = rtc.sdp_api();
        let mid = api.add_media(
            MediaKind::Audio,
            Direction::SendOnly,
            Some("ethereal".into()),
            Some("master".into()),
            None,
        );
        let (offer, pending) = api.apply().expect("adding media needs an offer");
        let offset = getrandom::u32().unwrap_or(0x5eed_1234);
        let peer = Self {
            listener,
            stream,
            rtc,
            mid,
            pending: Some(pending),
            offset,
            connected: false,
            created: now,
            disconnected_since: None,
            next_timeout: now,
            last_anchor_n: None,
            estimate: None,
            stun_pending: 0,
            eoc_sent: false,
            done: None,
        };
        (peer, offer.to_sdp_string())
    }

    pub fn key(&self) -> (SiteId, u32) {
        (self.listener, self.stream)
    }

    fn signal(&self, signal: StreamSignal) -> StreamOutput {
        StreamOutput::Signal {
            listener: self.listener,
            stream: self.stream,
            signal,
        }
    }

    fn ice(&self, candidate: String) -> StreamOutput {
        self.signal(StreamSignal::Ice {
            candidate: IceCandidate {
                candidate,
                sdp_mid: Some(self.mid.to_string()),
                sdp_m_line_index: Some(0),
                username_fragment: None,
            },
        })
    }

    fn fail(&mut self, reason: impl Into<String>) {
        if self.done.is_none() {
            self.done = Some(StreamLinkState::Failed {
                reason: reason.into(),
            });
        }
    }

    /// Emit the end-of-candidates marker once srflx gathering is over.
    pub fn maybe_end_of_candidates(&mut self, out: &mut Vec<StreamOutput>) {
        if self.stun_pending == 0 && !self.eoc_sent {
            self.eoc_sent = true;
            out.push(self.ice(String::new()));
        }
    }

    /// Add a server-reflexive candidate (STUN result) and trickle it.
    pub fn add_srflx(&mut self, mapped: SocketAddr, base: SocketAddr, out: &mut Vec<StreamOutput>) {
        if mapped == base {
            return; // no NAT: the host candidate already covers it
        }
        let Ok(c) = Candidate::server_reflexive(mapped, base, "udp") else {
            return;
        };
        if let Some(c) = self.rtc.add_local_candidate(c) {
            let s = c.to_sdp_string();
            out.push(self.ice(s));
        }
    }

    /// A signal from the listener.
    pub fn on_signal(&mut self, signal: &StreamSignal, now: Instant) {
        match signal {
            StreamSignal::Answer { sdp } => {
                let Some(pending) = self.pending.take() else {
                    return; // a second answer: ignore
                };
                let answer = match SdpAnswer::from_sdp_string(sdp) {
                    Ok(a) => a,
                    Err(e) => return self.fail(format!("bad answer: {e}")),
                };
                if let Err(e) = self.rtc.sdp_api().accept_answer(pending, answer) {
                    self.fail(format!("answer rejected: {e}"));
                }
            }
            StreamSignal::Ice { candidate } => {
                let s = candidate.candidate.trim();
                let s = s.strip_prefix("a=").unwrap_or(s);
                if s.is_empty() {
                    return; // end of candidates
                }
                // Unparsable (e.g. mDNS `.local` names): ignored, prflx covers them.
                if let Ok(c) = Candidate::from_sdp_string(s) {
                    self.rtc.add_remote_candidate(c);
                }
            }
            StreamSignal::Bye { .. } => {
                let _ = self.rtc.close();
                self.done = Some(StreamLinkState::Closed);
            }
            StreamSignal::Offer { .. } => {} // the host is always the offerer
        }
        self.next_timeout = now;
    }

    /// Queue one Opus frame whose first decoded sample is stream sample `n` (`ts_base`:
    /// `n` minus the codec delay, plus 2^33 so it never underflows).
    pub fn write(&mut self, data: Arc<[u8]>, ts_base: u64, now: Instant) {
        if !self.connected || self.done.is_some() {
            return;
        }
        let t = MediaTime::new(self.offset as u64 + ts_base, Frequency::FORTY_EIGHT_KHZ);
        if let Some(w) = self.rtc.writer(self.mid)
            && let Err(e) = w.write(Pt::from(OPUS_PT), now, t, data)
        {
            tracing::debug!("stream write: {e}");
        }
    }

    /// Feed a datagram.
    pub fn receive(&mut self, input: Input<'_>) {
        if let Err(e) = self.rtc.handle_input(input) {
            tracing::debug!("stream input: {e}");
        }
    }

    /// Drive time forward if due.
    pub fn timeout(&mut self, now: Instant) {
        if now >= self.next_timeout
            && let Err(e) = self.rtc.handle_input(Input::Timeout(now))
        {
            self.fail(e.to_string());
        }
    }

    /// Poll the `Rtc` until it wants time: send its datagrams, turn its events into outputs.
    pub fn drive(&mut self, socket: &UdpSocket, now: Instant, out: &mut Vec<StreamOutput>) {
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(t)) => {
                    self.next_timeout = t;
                    break;
                }
                Ok(Output::Transmit(t)) => {
                    // Unreachable destinations (e.g. IPv6 candidates on our IPv4 socket)
                    // fail here: ICE picks another pair.
                    let _ = socket.send_to(&t.contents, t.destination);
                }
                Ok(Output::Event(e)) => self.on_event(e, now, out),
                Err(e) => {
                    self.fail(e.to_string());
                    break;
                }
            }
        }
        if self.done.is_none() {
            if !self.rtc.is_alive() {
                self.fail("connection closed");
            } else if let Some(since) = self.disconnected_since {
                if now.duration_since(since) >= DISCONNECTED_FAIL {
                    self.fail("the listener stopped answering (ICE disconnected)");
                }
            } else if !self.connected && now.duration_since(self.created) >= CONNECT_TIMEOUT {
                self.fail("could not connect to the listener (ICE/DTLS timed out)");
            }
        }
        if let Some(state) = &self.done {
            out.push(StreamOutput::State {
                listener: self.listener,
                stream: self.stream,
                state: state.clone(),
            });
        }
    }

    fn on_event(&mut self, e: Event, now: Instant, out: &mut Vec<StreamOutput>) {
        match e {
            Event::Connected => {
                if !self.connected {
                    self.connected = true;
                    out.push(StreamOutput::State {
                        listener: self.listener,
                        stream: self.stream,
                        state: StreamLinkState::Connected,
                    });
                }
            }
            Event::IceConnectionStateChange(s) => match s {
                IceConnectionState::Disconnected => {
                    self.disconnected_since.get_or_insert(now);
                }
                IceConnectionState::Connected | IceConnectionState::Completed => {
                    self.disconnected_since = None;
                }
                _ => {}
            },
            Event::EgressBitrateEstimate(
                BweKind::Twcc { estimate, .. } | BweKind::Remb { estimate, .. },
            ) => self.estimate = Some(estimate.as_u64()),
            Event::Closed if self.done.is_none() => self.done = Some(StreamLinkState::Closed),
            _ => {}
        }
    }
}
