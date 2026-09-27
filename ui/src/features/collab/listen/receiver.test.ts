import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamSignal } from "@/generated";
import { FakePeerConnection, fakePeerConnection, fakeSink } from "./fakeRtc";
import { fromWireCandidate, ListenReceiver, NO_MEDIA_TIMEOUT_MS, STATS_INTERVAL_MS, toWireCandidate, webrtcUnsupportedReason } from "./receiver";

function setup() {
  const sent: StreamSignal[] = [];
  const failed: string[] = [];
  const sink = fakeSink();
  let now = 1_000_000;
  const r = new ListenReceiver({
    host: "9",
    stream: 42,
    iceServers: [
      { urls: ["stun:relay.test:9003"], username: null, credential: null },
      {
        urls: ["turn:relay.test:9003?transport=udp"],
        username: "123:1",
        credential: "c",
      },
    ],
    sink,
    send: (s) => sent.push(s),
    onFailed: (reason) => failed.push(reason),
    createPeerConnection: fakePeerConnection,
    now: () => now,
  });
  const pc = FakePeerConnection.last!;
  return {
    r,
    pc,
    sent,
    failed,
    sink,
    advance: (ms: number) => (now += ms),
    nowMs: () => now,
  };
}

const candidate = {
  candidate: "candidate:1 1 udp 1 10.0.0.2 5000 typ host",
  sdp_mid: "0",
  sdp_m_line_index: 0,
  username_fragment: null,
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("ListenReceiver", () => {
  it("uses the advertised ICE servers", () => {
    const { pc, r } = setup();
    expect(pc.config.iceServers).toEqual([
      { urls: ["stun:relay.test:9003"] },
      {
        urls: ["turn:relay.test:9003?transport=udp"],
        username: "123:1",
        credential: "c",
      },
    ]);
    r.close();
  });

  it("answers the host's offer, trickles ICE both ways (queued before the offer)", async () => {
    const { r, pc, sent } = setup();
    await r.onSignal({ type: "Ice", candidate });
    expect(pc.candidates).toEqual([]); // queued until the remote description
    await r.onSignal({ type: "Offer", sdp: "v=0 host" });
    expect(sent).toEqual([{ type: "Answer", sdp: "answer to v=0 host" }]);
    expect(pc.candidates).toEqual([fromWireCandidate(candidate)]);
    await r.onSignal({ type: "Ice", candidate });
    expect(pc.candidates).toHaveLength(2);

    pc.emitCandidate({
      candidate: "candidate:2",
      sdpMid: "0",
      sdpMLineIndex: 0,
    });
    pc.emitCandidate(null); // end of candidates
    expect(sent.slice(1)).toEqual([
      {
        type: "Ice",
        candidate: {
          candidate: "candidate:2",
          sdp_mid: "0",
          sdp_m_line_index: 0,
          username_fragment: null,
        },
      },
      {
        type: "Ice",
        candidate: {
          candidate: "",
          sdp_mid: null,
          sdp_m_line_index: null,
          username_fragment: null,
        },
      },
    ]);
    r.close();
  });

  it("plays the track and reports the playout RTP timestamp", () => {
    const { r, pc, sink, nowMs } = setup();
    expect(r.playoutRtp()).toBeNull();
    pc.emitTrack("the stream");
    expect(sink.played).toEqual(["the stream"]);
    expect(r.playoutRtp()).toBeNull();
    // Delivered 20 ms ago, 5 ms output latency: +15 ms = 720 samples.
    sink.latency = 0.005;
    pc.receiver.sources = [{ source: 1, rtpTimestamp: 90_000, timestamp: nowMs() - 20 }];
    expect(r.playoutRtp()).toBe(90_720);
    r.close();
  });

  it("says Bye when the connection fails or on a bad offer", async () => {
    const a = setup();
    a.pc.setState("failed");
    expect(a.sent).toEqual([{ type: "Bye", reason: "the connection failed" }]);
    expect(a.failed).toEqual(["the connection failed"]);
    expect(a.pc.closed && a.sink.closed && a.r.isClosed).toBe(true);
    a.r.fail("again");
    expect(a.sent).toHaveLength(1);

    const b = setup();
    await b.r.onSignal({ type: "Offer", sdp: "bad" });
    expect(b.sent).toEqual([{ type: "Bye", reason: "could not connect: invalid SDP" }]);
  });

  it("gives up without media for 10 s, estimates the receive delay from stats", async () => {
    const { r, pc, sent, advance } = setup();
    pc.stats = [
      {
        type: "inbound-rtp",
        kind: "audio",
        packetsReceived: 10,
        jitterBufferDelay: 6,
        jitterBufferEmittedCount: 100,
      },
      { type: "candidate-pair", nominated: true, currentRoundTripTime: 0.04 },
    ];
    advance(STATS_INTERVAL_MS);
    await vi.advanceTimersByTimeAsync(STATS_INTERVAL_MS);
    expect(r.receiveDelaySec()).toBeCloseTo(0.06 + 0.02, 9);
    // Packets stop arriving.
    advance(NO_MEDIA_TIMEOUT_MS + 1);
    await vi.advanceTimersByTimeAsync(STATS_INTERVAL_MS);
    expect(sent).toEqual([{ type: "Bye", reason: "no audio from the host for 10 s" }]);
    expect(r.isClosed).toBe(true);
  });

  it("converts candidates and detects WebRTC", () => {
    expect(toWireCandidate(fromWireCandidate(candidate))).toEqual(candidate);
    const saved = globalThis.RTCPeerConnection;
    // @ts-expect-error: simulate a webview without WebRTC (WebKitGTK)
    delete globalThis.RTCPeerConnection;
    expect(webrtcUnsupportedReason()).toMatch(/no WebRTC/);
    globalThis.RTCPeerConnection = FakePeerConnection as unknown as typeof RTCPeerConnection;
    expect(webrtcUnsupportedReason()).toBeNull();
    if (saved) globalThis.RTCPeerConnection = saved;
    // @ts-expect-error: restore jsdom's state (no WebRTC)
    else delete globalThis.RTCPeerConnection;
  });
});
