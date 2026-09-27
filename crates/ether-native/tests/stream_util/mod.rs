//! Shared helpers of the `stream-host` tests: an in-test WebRTC listener (a str0m receiver
//! on its own UDP socket on 127.0.0.1, same crypto backend), the sender's output
//! collector, and an Opus decoder in RTP space.
#![allow(dead_code)]

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use ether_controller::streaming::{StreamLinkState, StreamOutput};
use ether_core::protocol::collab::{IceCandidate, StreamClock, StreamSignal};
use ether_core::protocol::model::SiteId;
use ether_native::stream::{SenderConfig, StreamHost, StreamSender};
use str0m::change::SdpOffer;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

pub const LISTENER: SiteId = SiteId(7);
pub const STREAM: u32 = 42;

/// What [`connect`] needs from the host side: the sender itself, or the bridge-facing
/// `StreamHost`.
pub trait Endpoint {
    fn open(&mut self);
    fn signal(&mut self, s: StreamSignal);
    fn poll(&mut self, out: &mut Vec<StreamOutput>);
}

impl Endpoint for StreamSender {
    fn open(&mut self) {
        StreamSender::open(self, LISTENER, STREAM, &[]).unwrap();
    }
    fn signal(&mut self, s: StreamSignal) {
        StreamSender::signal(self, LISTENER, STREAM, &s).unwrap();
    }
    fn poll(&mut self, out: &mut Vec<StreamOutput>) {
        StreamSender::poll(self, out);
    }
}

impl Endpoint for StreamHost {
    fn open(&mut self) {
        StreamHost::open(self, LISTENER, STREAM, &[]).unwrap();
    }
    fn signal(&mut self, s: StreamSignal) {
        StreamHost::signal(self, LISTENER, STREAM, &s).unwrap();
    }
    fn poll(&mut self, out: &mut Vec<StreamOutput>) {
        StreamHost::poll(self, out);
    }
}

/// A loopback-only sender (deterministic candidates).
pub fn loopback_sender() -> StreamSender {
    StreamSender::spawn(SenderConfig {
        loopback: true,
        default_route: false,
    })
    .expect("sender")
}

/// One received RTP packet.
#[derive(Clone, Debug)]
pub struct Packet {
    pub seq: u16,
    pub ts: u32,
    pub payload: Vec<u8>,
}

/// The receiving end (what a listener's `RTCPeerConnection` does), driven by the test.
pub struct Listener {
    pub rtc: Rtc,
    socket: UdpSocket,
    addr: SocketAddr,
    next: Instant,
    pub connected: bool,
    pub packets: Vec<Packet>,
    buf: Vec<u8>,
}

impl Listener {
    pub fn new() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let now = Instant::now();
        // rtp_mode: RTP packets surface with their wire header (timestamp).
        let mut rtc = Rtc::builder().set_rtp_mode(true).build(now);
        rtc.add_local_candidate(Candidate::host(addr, "udp").unwrap());
        Self {
            rtc,
            socket,
            addr,
            next: now,
            connected: false,
            packets: Vec::new(),
            buf: vec![0; 2048],
        }
    }

    /// Accept the host's offer; returns the answer SDP (our candidate embedded).
    pub fn answer(&mut self, offer: &str) -> String {
        let offer = SdpOffer::from_sdp_string(offer).expect("offer parses");
        let answer = self
            .rtc
            .sdp_api()
            .accept_offer(offer)
            .expect("offer accepted");
        answer.to_sdp_string()
    }

    pub fn add_remote(&mut self, c: &IceCandidate) {
        if !c.candidate.is_empty() {
            self.rtc
                .add_remote_candidate(Candidate::from_sdp_string(&c.candidate).unwrap());
        }
    }

    /// Drive the receiver for at most `max` (one socket read).
    pub fn pump(&mut self, max: Duration) {
        loop {
            match self.rtc.poll_output().expect("receiver poll") {
                Output::Timeout(t) => {
                    self.next = t;
                    break;
                }
                Output::Transmit(t) => {
                    let _ = self.socket.send_to(&t.contents, t.destination);
                }
                Output::Event(e) => match e {
                    Event::Connected => self.connected = true,
                    Event::RtpPacket(p) => self.packets.push(Packet {
                        seq: p.header.sequence_number,
                        ts: p.header.timestamp,
                        payload: p.payload.to_vec(),
                    }),
                    _ => {}
                },
            }
        }
        let now = Instant::now();
        let wait = self
            .next
            .saturating_duration_since(now)
            .min(max)
            .max(Duration::from_micros(200));
        self.socket.set_read_timeout(Some(wait)).unwrap();
        match self.socket.recv_from(&mut self.buf) {
            Ok((n, source)) => {
                let input = Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: self.addr,
                        contents: self.buf[..n].try_into().unwrap(),
                    },
                );
                self.rtc.handle_input(input).expect("receiver input");
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => panic!("recv: {e}"),
        }
        let now = Instant::now();
        if now >= self.next {
            self.rtc.handle_input(Input::Timeout(now)).unwrap();
        }
    }
}

/// Everything the sender produced, in order.
#[derive(Default)]
pub struct Outputs {
    pub all: Vec<StreamOutput>,
    seen: usize,
}

impl Outputs {
    pub fn collect(&mut self, ep: &mut impl Endpoint) {
        ep.poll(&mut self.all);
    }

    /// The outputs since the last `fresh` call.
    pub fn fresh(&mut self) -> Vec<StreamOutput> {
        let v = self.all[self.seen..].to_vec();
        self.seen = self.all.len();
        v
    }

    pub fn clocks(&self) -> Vec<StreamClock> {
        self.all
            .iter()
            .filter_map(|o| match o {
                StreamOutput::Clock {
                    listener,
                    stream,
                    clock,
                } => {
                    assert_eq!((*listener, *stream), (LISTENER, STREAM));
                    Some(clock.clone())
                }
                _ => None,
            })
            .collect()
    }

    pub fn states(&self) -> Vec<StreamLinkState> {
        self.all
            .iter()
            .filter_map(|o| match o {
                StreamOutput::State { state, .. } => Some(state.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn signals(&self) -> Vec<StreamSignal> {
        self.all
            .iter()
            .filter_map(|o| match o {
                StreamOutput::Signal { signal, .. } => Some(signal.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Open a stream to a fresh [`Listener`] and wait until media can flow.
pub fn connect(ep: &mut impl Endpoint, out: &mut Outputs, mut tick: impl FnMut()) -> Listener {
    ep.open();
    let mut l = Listener::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut answered = false;
    loop {
        assert!(Instant::now() < deadline, "no connection: {:?}", out.all);
        out.collect(ep);
        for o in out.fresh() {
            match o {
                StreamOutput::Signal {
                    signal: StreamSignal::Offer { sdp },
                    ..
                } => {
                    assert!(sdp.contains("a=sendonly"), "{sdp}");
                    assert!(sdp.contains("opus/48000/2"), "{sdp}");
                    assert!(sdp.contains("stereo=1"), "{sdp}");
                    assert!(sdp.contains("sprop-stereo=1"), "{sdp}");
                    assert!(sdp.contains("useinbandfec=1"), "{sdp}");
                    assert!(
                        sdp.contains("127.0.0.1"),
                        "host candidate in the offer: {sdp}"
                    );
                    let answer = l.answer(&sdp);
                    ep.signal(StreamSignal::Answer { sdp: answer });
                    answered = true;
                }
                StreamOutput::Signal {
                    signal: StreamSignal::Ice { candidate },
                    ..
                } => l.add_remote(&candidate),
                StreamOutput::State {
                    state: StreamLinkState::Failed { reason },
                    ..
                } => panic!("failed: {reason}"),
                _ => {}
            }
        }
        if answered {
            l.pump(Duration::from_millis(2));
        }
        tick();
        if l.connected && out.states().contains(&StreamLinkState::Connected) {
            return l;
        }
    }
}

/// Decoded audio in RTP space: `(rtp of the first sample, left, right)` per packet, in
/// sequence order (warm-up packets skipped).
pub fn decode(packets: &[Packet], skip: usize) -> Vec<(u32, Vec<f32>, Vec<f32>)> {
    let mut ps = packets.to_vec();
    ps.sort_by_key(|p| p.seq);
    ps.dedup_by_key(|p| p.seq);
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Stereo).unwrap();
    let mut out = vec![0f32; 2 * 5760];
    let mut v = Vec::new();
    for (i, p) in ps.iter().enumerate() {
        let n = dec.decode_float(&p.payload, &mut out, false).unwrap();
        if i >= skip {
            let l = (0..n).map(|k| out[2 * k]).collect();
            let r = (0..n).map(|k| out[2 * k + 1]).collect();
            v.push((p.ts, l, r));
        }
    }
    v
}
