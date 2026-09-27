/** The web sender with a scripted RTCPeerConnection (docs/COLLAB.md §9.1-§9.4). */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CollabCommand, ListenerLink, StreamClock, StreamSignal } from "@/generated";
import { CONNECT_TIMEOUT_MS, WebSender, type LinkState } from "./sender";
import { createFakePeer, FakeObserver, FakePeerConnection, fakeStream, FakeTimeline } from "./testing";

const ui = (site: string, stream: number): ListenerLink => ({ site, stream, endpoint: "Ui" });

let sent: CollabCommand[];
let now: number;
let timeline: FakeTimeline;
let observer: FakeObserver;
let links: ReadonlyMap<string, LinkState>;
let sender: WebSender;

const signals = (to?: string) =>
  sent.flatMap((c) => (c.type === "SendSignal" && (to === undefined || c.to === to) ? [c.signal] : [])) as StreamSignal[];
const clocks = (to?: string) =>
  sent.flatMap((c) => (c.type === "SendStreamClock" && (to === undefined || c.to === to) ? [c.clock] : [])) as StreamClock[];
const pcs = () => FakePeerConnection.created;
const settle = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => {
  vi.useFakeTimers();
  FakePeerConnection.created = [];
  sent = [];
  now = 0;
  timeline = new FakeTimeline();
  observer = new FakeObserver();
  links = new Map();
  sender = new WebSender({
    send: (c) => {
      sent.push(c);
      return Promise.resolve();
    },
    createPeer: createFakePeer,
    stream: fakeStream(),
    observer,
    timeline,
    onChange: (l) => (links = l),
    now: () => now,
  });
});

afterEach(() => {
  sender.dispose();
  vi.useRealTimers();
});

describe("WebSender signaling", () => {
  it("opens one connection per Ui listener with a munged offer and trickles ICE", async () => {
    sender.setIceServers([{ urls: ["stun:relay:3478"], username: null, credential: null }, { urls: ["turn:relay:3478"], username: "u", credential: "p" }]);
    sender.setListeners([ui("2", 7), { site: "3", stream: 8, endpoint: "Engine" }, ui("4", 9)]);
    await settle();
    expect(pcs()).toHaveLength(2);
    expect(pcs()[0]!.config).toEqual({
      iceServers: [{ urls: ["stun:relay:3478"] }, { urls: ["turn:relay:3478"], username: "u", credential: "p" }],
      encodedInsertableStreams: true,
    });
    expect(pcs()[0]!.tracks[0]!.contentHint).toBe("music");
    const offer = signals("2")[0];
    expect(offer).toMatchObject({ type: "Offer" });
    expect(offer?.type === "Offer" && offer.sdp).toContain("stereo=1;sprop-stereo=1;maxaveragebitrate=128000;useinbandfec=1");
    expect(pcs()[0]!.localDescription?.sdp).toBe(offer?.type === "Offer" ? offer.sdp : "");
    expect(sent.find((c) => c.type === "SendSignal" && c.to === "4")).toMatchObject({ stream: 9 });
    expect([...links]).toEqual([
      ["2:7", "connecting"],
      ["4:9", "connecting"],
    ]);

    pcs()[0]!.gather("candidate:1 1 udp 2122260223 10.0.0.1 5000 typ host");
    pcs()[0]!.gather(null);
    expect(signals("2").slice(1)).toEqual([
      {
        type: "Ice",
        candidate: { candidate: "candidate:1 1 udp 2122260223 10.0.0.1 5000 typ host", sdp_mid: "0", sdp_m_line_index: 0, username_fragment: "uf" },
      },
      { type: "Ice", candidate: { candidate: "", sdp_mid: null, sdp_m_line_index: null, username_fragment: null } },
    ]);
  });

  it("applies the answer, queues early ICE, and closes on Bye; stale signals are ignored", async () => {
    sender.setListeners([ui("2", 7)]);
    await settle();
    const pc = pcs()[0]!;
    const ice = { candidate: "candidate:2", sdp_mid: "0", sdp_m_line_index: 0, username_fragment: null };
    sender.onSignal("2", 7, { type: "Ice", candidate: ice });
    expect(pc.candidates).toEqual([]);
    sender.onSignal("2", 99, { type: "Answer", sdp: "stale" });
    sender.onSignal("2", 7, { type: "Answer", sdp: "answer-sdp" });
    await settle();
    expect(pc.remoteDescription).toEqual({ type: "answer", sdp: "answer-sdp" });
    expect(pc.candidates).toEqual([{ candidate: "candidate:2", sdpMid: "0", sdpMLineIndex: 0, usernameFragment: null }]);
    sender.onSignal("2", 7, { type: "Ice", candidate: { ...ice, candidate: "" } });
    expect(pc.candidates.at(-1)).toMatchObject({ candidate: "" });

    sender.onSignal("2", 7, { type: "Bye", reason: "stopped" });
    expect(pc.closed).toBe(true);
    expect(signals().filter((s) => s.type === "Bye")).toEqual([]);
    // Still listed until the controller drops it: not reopened.
    sender.setListeners([ui("2", 7)]);
    expect(pcs()).toHaveLength(1);
    // A new request (new stream id) opens a new connection.
    sender.setListeners([ui("2", 8)]);
    expect(pcs()).toHaveLength(2);
  });

  it("closes connections whose listener disappears, without a Bye", async () => {
    sender.setListeners([ui("2", 7), ui("3", 8)]);
    await settle();
    sender.setListeners([ui("3", 8)]);
    expect(pcs()[0]!.closed).toBe(true);
    expect(pcs()[1]!.closed).toBe(false);
    expect([...links.keys()]).toEqual(["3:8"]);
    expect(signals().some((s) => s.type === "Bye")).toBe(false);
    sender.dispose();
    expect(pcs()[1]!.closed).toBe(true);
    expect(observer.disposed).toBe(true);
  });

  it("sends Bye and closes when the connection fails, times out or the answer is bad", async () => {
    sender.setListeners([ui("2", 7), ui("3", 8), ui("4", 9)]);
    await settle();
    pcs()[0]!.setState("failed");
    expect(pcs()[0]!.closed).toBe(true);
    expect(signals("2").at(-1)).toEqual({ type: "Bye", reason: "connection failed" });

    pcs()[2]!.rejectAnswer = true;
    sender.onSignal("4", 9, { type: "Answer", sdp: "garbage" });
    await settle();
    expect(signals("4").at(-1)).toMatchObject({ type: "Bye" });
    expect(pcs()[2]!.closed).toBe(true);

    await vi.advanceTimersByTimeAsync(CONNECT_TIMEOUT_MS);
    expect(signals("3").at(-1)).toEqual({ type: "Bye", reason: "no connection to the listener" });
    expect(pcs()[1]!.closed).toBe(true);
    // Ended links are not reopened while listed.
    sender.setListeners([ui("2", 7), ui("3", 8), ui("4", 9)]);
    expect(pcs()).toHaveLength(3);
  });
});

describe("WebSender anchors", () => {
  async function connected(site = "2", stream = 7) {
    sender.setListeners([ui(site, stream)]);
    await settle();
    pcs().at(-1)!.setState("connected");
  }

  it("sends nothing before media flows, then a discontinuity, then every 100 ms", async () => {
    timeline.set({ tapFrame: 10_000, position: 2 });
    sender.setListeners([ui("2", 7)]);
    await settle();
    // Frames observed while connecting: offset known, but not connected yet.
    observer.emit(0, 5_000, 100); // context frame 4_800 → offset 200
    expect(clocks()).toEqual([]);
    pcs()[0]!.setState("connected");
    expect(links.get("2:7")).toBe("connected");
    expect(clocks()).toEqual([
      {
        rtp: 10_200,
        position: 2,
        playing: true,
        recording: false,
        bpm: 120,
        loop_enabled: false,
        loop_region: { start: 0, end: 16 },
        metronome: true,
        discontinuity: true,
      },
    ]);
    expect(sent.at(-1)).toMatchObject({ type: "SendStreamClock", to: "2", stream: 7 });

    now = 50;
    timeline.set({ tapFrame: 12_400, position: 2.1 });
    sender.tick();
    expect(clocks()).toHaveLength(1);
    now = 100;
    sender.tick();
    expect(clocks().at(-1)).toMatchObject({ rtp: 12_600, position: 2.1, discontinuity: false });
  });

  it("waits for the first encoded frame (no RTP offset yet)", async () => {
    timeline.set({ tapFrame: 10_000 });
    await connected();
    expect(clocks()).toEqual([]);
    observer.emit(0, 1_000, 0);
    expect(clocks()).toEqual([expect.objectContaining({ rtp: 11_000, discontinuity: true })]);
  });

  it("sends a jump at once, anchored at its tap frame (render + latency)", async () => {
    timeline.set({ tapFrame: 10_000, position: 7.8, latency: 480 });
    await connected();
    observer.emit(0, 1_000_000, 0);
    now = 30;
    // Loop wrap rendered at 20_000: heard at 20_480; the latest block is already later.
    timeline.set({ tapFrame: 21_000, position: 0.02, latency: 480 }, { tapFrame: 20_480, position: 0, latency: 480 });
    sender.tick();
    expect(clocks().slice(1)).toEqual([expect.objectContaining({ rtp: 1_020_480, position: 0, discontinuity: true })]);
    // Stop: an event with playing false.
    now = 60;
    timeline.set({ tapFrame: 22_000, position: 0.5, playing: false, latency: 480 }, { tapFrame: 21_500, position: 0.5, playing: false, latency: 480 });
    sender.tick();
    expect(clocks().at(-1)).toMatchObject({ rtp: 1_021_500, playing: false, discontinuity: true });
    expect(clocks()).toHaveLength(3);
  });

  it("keeps per-listener RTP offsets and wraps at 2^32", async () => {
    timeline.set({ tapFrame: 1_000 });
    sender.setListeners([ui("2", 7), ui("3", 8)]);
    await settle();
    pcs()[0]!.setState("connected");
    pcs()[1]!.setState("connected");
    observer.emit(0, 2 ** 32 - 500, 0);
    observer.emit(1, 42, 0);
    expect(clocks("2").at(-1)?.rtp).toBe(500);
    expect(clocks("3").at(-1)?.rtp).toBe(1_042);
  });

  it("carries count_in_end during a count-in", async () => {
    timeline.preRoll = 4;
    timeline.set({ tapFrame: 1_000, playing: false, recording: true, position: 4 });
    await connected();
    observer.emit(0, 0, 0);
    expect(clocks().at(-1)).not.toHaveProperty("count_in_end");
    now = 20;
    timeline.set({ tapFrame: 2_000, recording: true, position: 4.01 }, { tapFrame: 1_900, recording: true, position: 4 });
    sender.tick();
    expect(clocks().at(-1)).toMatchObject({ position: 4, recording: true, count_in_end: 8, discontinuity: true });
  });
});
