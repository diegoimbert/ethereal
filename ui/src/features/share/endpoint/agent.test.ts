import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CONNECT_TIMEOUT_MS, FLOW_POLL_MS, LOW_WATER_BYTES, sdpFingerprint, startShareEndpoint } from "./agent";
import { FakePeerConnection, FakePort, fakePeerConnection } from "./fakeRtc";
import { DC_LABEL, type FromUi } from "./protocol";

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

const STUN = { urls: ["stun:stun.example:3478"], username: null, credential: null };
const TURN = { urls: ["turn:turn.example:3478"], username: "u", credential: "p" };

function setup() {
  const port = new FakePort();
  const stop = startShareEndpoint(port, { createPeerConnection: fakePeerConnection });
  return { port, stop };
}

const last = () => FakePeerConnection.all[FakePeerConnection.all.length - 1]!;
const ofType = <T extends FromUi["type"]>(port: FakePort, type: T) =>
  port.messages().filter((m): m is Extract<FromUi, { type: T }> => m.type === type);

beforeEach(() => {
  FakePeerConnection.all = [];
});
afterEach(() => {
  vi.useRealTimers();
});

describe("sdpFingerprint", () => {
  it("normalises the a=fingerprint line", () => {
    expect(sdpFingerprint("v=0\r\na=fingerprint:SHA-256 ab:0c:FF\r\n")).toBe("sha-256 AB:0C:FF");
    expect(sdpFingerprint("v=0\r\n")).toBeNull();
    expect(sdpFingerprint(undefined)).toBeNull();
  });
});

describe("share endpoint agent", () => {
  it("joiner: data channel, offer, trickle, answer, open with fingerprints", async () => {
    const { port } = setup();
    port.deliver({ type: "open", ch: 3, offer: true, ice: [STUN, TURN], relay: false });
    const pc = last();
    expect(pc.config.iceServers).toEqual([{ urls: STUN.urls }, { urls: TURN.urls, username: "u", credential: "p" }]);
    expect(pc.config.iceTransportPolicy).toBe("all");
    expect(pc.channels.map((c) => c.label)).toEqual([DC_LABEL]);
    await flush();
    expect(ofType(port, "signal")[0]).toEqual({ type: "signal", ch: 3, signal: { type: "Offer", sdp: pc.localDescription!.sdp } });

    pc.emitCandidate({ candidate: "candidate:1 1 udp 1 10.0.0.1 5000 typ host", sdpMid: "0", sdpMLineIndex: 0 });
    pc.emitCandidate(null);
    const ice = ofType(port, "signal").slice(1).map((m) => m.signal);
    expect(ice).toEqual([
      {
        type: "Ice",
        candidate: { candidate: "candidate:1 1 udp 1 10.0.0.1 5000 typ host", sdp_mid: "0", sdp_m_line_index: 0, username_fragment: null },
      },
      { type: "Ice", candidate: { candidate: "", sdp_mid: null, sdp_m_line_index: null, username_fragment: null } },
    ]);

    // A remote candidate before the answer waits for it; end-of-candidates is not added.
    const cand = { candidate: "candidate:2 1 udp 1 10.0.0.2 6000 typ host", sdp_mid: null, sdp_m_line_index: null, username_fragment: null };
    port.deliver({ type: "signal", ch: 3, signal: { type: "Ice", candidate: cand } });
    port.deliver({ type: "signal", ch: 3, signal: { type: "Ice", candidate: { ...cand, candidate: "" } } });
    expect(pc.candidates).toEqual([]);
    port.deliver({ type: "signal", ch: 3, signal: { type: "Answer", sdp: "v=0\r\na=fingerprint:sha-256 ee:ff\r\n" } });
    await flush();
    expect(pc.remoteDescription).toEqual({ type: "answer", sdp: "v=0\r\na=fingerprint:sha-256 ee:ff\r\n" });
    expect(pc.candidates).toEqual([{ candidate: cand.candidate, sdpMid: null, sdpMLineIndex: 0, usernameFragment: null }]);

    pc.channels[0]!.open();
    expect(ofType(port, "open")).toEqual([{ type: "open", ch: 3, local: "sha-256 AA:BB", remote: "sha-256 EE:FF" }]);
    expect(pc.channels[0]!.binaryType).toBe("arraybuffer");
    expect(pc.channels[0]!.bufferedAmountLowThreshold).toBe(LOW_WATER_BYTES);
  });

  it("host: waits for the offer, answers, takes the joiner's channel", async () => {
    const { port } = setup();
    port.deliver({ type: "open", ch: 1, offer: false, ice: [], relay: false });
    const pc = last();
    expect(pc.channels).toEqual([]);
    port.deliver({ type: "signal", ch: 1, signal: { type: "Offer", sdp: "v=0\r\na=fingerprint:sha-256 01:02\r\n" } });
    await flush();
    expect(ofType(port, "signal")).toEqual([{ type: "signal", ch: 1, signal: { type: "Answer", sdp: pc.localDescription!.sdp } }]);
    // A second offer (renegotiation) is ignored.
    port.deliver({ type: "signal", ch: 1, signal: { type: "Offer", sdp: "v=0" } });
    await flush();
    expect(ofType(port, "signal")).toHaveLength(1);

    pc.emitChannel("other");
    const dc = pc.emitChannel(DC_LABEL);
    dc.open();
    expect(ofType(port, "open")).toEqual([{ type: "open", ch: 1, local: "sha-256 CC:DD", remote: "sha-256 01:02" }]);
  });

  it("moves bytes both ways and reports flow", async () => {
    const { port } = setup();
    port.deliver({ type: "open", ch: 2, offer: true, ice: [], relay: false });
    const pc = last();
    port.deliver({ type: "signal", ch: 2, signal: { type: "Answer", sdp: "v=0\r\na=fingerprint:sha-256 ee:ff\r\n" } });
    await flush();
    const dc = pc.channels[0]!;
    // Before open, worker data is dropped (the Worker has no link yet anyway).
    port.deliver({ type: "data", ch: 2, data: new ArrayBuffer(4) });
    expect(dc.sent).toEqual([]);
    dc.open();

    port.deliver({ type: "data", ch: 2, data: new Uint8Array([1, 2, 3]).buffer });
    port.deliver({ type: "data", ch: 2, data: new Uint8Array([4]).buffer });
    expect(dc.sent.map((b) => [...new Uint8Array(b)])).toEqual([[1, 2, 3], [4]]);
    await flush();
    // One coalesced report per burst.
    expect(ofType(port, "flow")).toEqual([{ type: "flow", ch: 2, consumed: 4, buffered: 4 }]);
    // Under the low-water mark no event fires: the agent re-reports until it is empty.
    dc.bufferedAmount = 0;
    await vi.waitFor(() => expect(ofType(port, "flow").at(-1)).toEqual({ type: "flow", ch: 2, consumed: 4, buffered: 0 }));
    const reports = ofType(port, "flow").length;
    await new Promise((r) => setTimeout(r, FLOW_POLL_MS * 2));
    expect(ofType(port, "flow")).toHaveLength(reports);
    dc.bufferedAmount = 100_000;
    dc.drain();
    await flush();
    expect(ofType(port, "flow").at(-1)).toEqual({ type: "flow", ch: 2, consumed: 4, buffered: 0 });

    const incoming = new Uint8Array([9, 8]).buffer;
    dc.receive(incoming);
    dc.receive("text is not ours");
    const data = port.posted.filter((p) => p.message.type === "data");
    expect(data).toHaveLength(1);
    expect(data[0]!.message).toEqual({ type: "data", ch: 2, data: incoming });
    expect(data[0]!.transfer).toEqual([incoming]);

    dc.remoteClose();
    expect(ofType(port, "closed")).toEqual([{ type: "closed", ch: 2, reason: "the peer closed the data channel" }]);
    expect(pc.closed).toBe(true);
  });

  it("relay only: iceTransportPolicy relay", () => {
    const { port } = setup();
    port.deliver({ type: "open", ch: 1, offer: true, ice: [TURN], relay: true });
    expect(last().config.iceTransportPolicy).toBe("relay");
  });

  it("fails after the connect timeout, on ICE failure, on a bad offer, and on Bye", async () => {
    vi.useFakeTimers();
    const { port } = setup();
    port.deliver({ type: "open", ch: 1, offer: true, ice: [], relay: false });
    vi.advanceTimersByTime(CONNECT_TIMEOUT_MS - 1);
    expect(ofType(port, "closed")).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(ofType(port, "closed")).toEqual([{ type: "closed", ch: 1, reason: "could not connect to the peer (ICE/DTLS timed out)" }]);

    port.deliver({ type: "open", ch: 2, offer: true, ice: [], relay: false });
    last().setState("failed");
    expect(ofType(port, "closed")[1]).toEqual({ type: "closed", ch: 2, reason: "could not connect to the peer (ICE failed)" });

    port.deliver({ type: "open", ch: 3, offer: false, ice: [], relay: false });
    port.deliver({ type: "signal", ch: 3, signal: { type: "Offer", sdp: "bad" } });
    await vi.runAllTimersAsync();
    expect(ofType(port, "closed")[2]).toEqual({ type: "closed", ch: 3, reason: "bad offer: invalid SDP" });

    port.deliver({ type: "open", ch: 4, offer: false, ice: [], relay: false });
    port.deliver({ type: "signal", ch: 4, signal: { type: "Bye", reason: null } });
    expect(ofType(port, "closed")[3]).toEqual({ type: "closed", ch: 4, reason: "the peer ended the connection" });
    // A timer of a finished pairing does nothing.
    vi.advanceTimersByTime(CONNECT_TIMEOUT_MS);
    expect(ofType(port, "closed")).toHaveLength(4);
  });

  it("close from the Worker is silent; reopening a channel replaces it; dispose closes all", async () => {
    const { port, stop } = setup();
    port.deliver({ type: "open", ch: 1, offer: true, ice: [], relay: false });
    const first = last();
    port.deliver({ type: "close", ch: 1 });
    expect(first.closed).toBe(true);
    expect(first.channels[0]!.closed).toBe(true);
    port.deliver({ type: "open", ch: 2, offer: true, ice: [], relay: false });
    const second = last();
    port.deliver({ type: "open", ch: 2, offer: true, ice: [], relay: false });
    expect(second.closed).toBe(true);
    const third = last();
    stop();
    expect(third.closed).toBe(true);
    expect(port.onmessage).toBeNull();
    expect(ofType(port, "closed")).toEqual([]);
  });

  it("without WebRTC, pairings close at once", () => {
    const port = new FakePort();
    const saved = globalThis.RTCPeerConnection;
    // @ts-expect-error: removing the global for this test
    delete globalThis.RTCPeerConnection;
    try {
      startShareEndpoint(port);
      port.deliver({ type: "open", ch: 5, offer: true, ice: [], relay: false });
      expect(port.messages()).toEqual([{ type: "closed", ch: 5, reason: "this browser has no WebRTC" }]);
    } finally {
      if (saved) globalThis.RTCPeerConnection = saved;
    }
  });
});
