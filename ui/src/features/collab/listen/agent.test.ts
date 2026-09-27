import { afterEach, describe, expect, it } from "vitest";
import type { CollabCommand, CollabEvent, Event, PlayheadFrame, StreamClock } from "@/generated";
import { TempoMap } from "@/timeline/tempoMap";
import { ListenAgent } from "./agent";
import { FakePeerConnection, fakePeerConnection } from "./fakeRtc";
import { ListenReceiver } from "./receiver";
import { useListenStore } from "./store";

const ev = (event: CollabEvent): Event => ({ type: "Collab", event });

function clock(rtp: number, position: number, extra: Partial<StreamClock> = {}): StreamClock {
  return {
    rtp,
    position,
    playing: true,
    recording: false,
    bpm: 120,
    loop_enabled: false,
    loop_region: { start: 0, end: 8 },
    metronome: false,
    discontinuity: true,
    ...extra,
  };
}

function setup() {
  const sent: CollabCommand[] = [];
  const frames: (PlayheadFrame | null)[] = [];
  const callbacks: (() => void)[] = [];
  let now = 5_000;
  const agent = new ListenAgent({
    send: async (c) => {
      sent.push(c);
    },
    tempo: () => TempoMap.constant(120),
    setPlayhead: (f) => frames.push(f),
    createReceiver: (o) =>
      new ListenReceiver({
        ...o,
        createPeerConnection: fakePeerConnection,
        now: () => now,
      }),
    requestFrame: (cb) => callbacks.push(cb),
    cancelFrame: () => undefined,
    now: () => now,
  });
  return {
    agent,
    sent,
    frames,
    callbacks,
    advance: (ms: number) => (now += ms),
    nowMs: () => now,
  };
}

afterEach(() => useListenStore.getState().reset());

describe("ListenAgent", () => {
  it("runs a receiver for the current stream and maps its clock onto the playhead", async () => {
    const { agent, sent, frames, callbacks, nowMs } = setup();
    agent.onEvent(
      ev({
        type: "IceServers",
        servers: [{ urls: ["stun:r:1"], username: null, credential: null }],
        source: "Relay",
      }),
    );
    agent.onEvent(
      ev({
        type: "ListenStatus",
        status: {
          listening: { type: "Connecting", host: "9", stream: 7 },
          listeners: [],
        },
      }),
    );
    const pc = FakePeerConnection.last!;
    expect(pc.config.iceServers).toEqual([{ urls: ["stun:r:1"] }]);
    expect(callbacks).toHaveLength(1);

    // Signals for another stream are ignored; ours are answered through SendSignal.
    agent.onEvent(
      ev({
        type: "Signal",
        from: "9",
        stream: 8,
        signal: { type: "Offer", sdp: "stale" },
      }),
    );
    agent.onEvent(
      ev({
        type: "Signal",
        from: "9",
        stream: 7,
        signal: { type: "Offer", sdp: "fresh" },
      }),
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(sent).toEqual([
      {
        type: "SendSignal",
        to: "9",
        stream: 7,
        signal: { type: "Answer", sdp: "answer to fresh" },
      },
    ]);

    // Media + clock: the playhead shows what is heard (0.5 s after the anchor = +1 beat).
    pc.emitTrack();
    pc.receiver.sources = [{ source: 1, rtpTimestamp: 1_000 + 24_000, timestamp: nowMs() }];
    agent.onEvent(ev({ type: "StreamClock", from: "9", stream: 7, clock: clock(1_000, 4) }));
    agent.onEvent(
      ev({
        type: "ListenStatus",
        status: {
          listening: { type: "Listening", host: "9", stream: 7 },
          listeners: [],
        },
      }),
    );
    expect(FakePeerConnection.last).toBe(pc); // same stream: same receiver
    callbacks.shift()!();
    expect(frames.at(-1)!.transport.position).toBeCloseTo(5, 9);
    expect(frames.at(-1)!.transport.playing).toBe(true);
    expect(frames.at(-1)!.transport.seconds).toBeCloseTo(2.5, 9);
    expect(callbacks).toHaveLength(1); // keeps looping

    // Count-in is flagged for the badge.
    agent.onEvent(
      ev({
        type: "StreamClock",
        from: "9",
        stream: 7,
        clock: clock(30_000, -2, { recording: true, count_in_end: 0 }),
      }),
    );
    pc.receiver.sources = [{ source: 1, rtpTimestamp: 30_000, timestamp: nowMs() }];
    callbacks.shift()!();
    expect(frames.at(-1)!.transport.position).toBe(-2);
    expect(useListenStore.getState().countIn).toBe(true);

    // The stream ends: receiver closed, back to the engine's playhead.
    agent.onEvent(
      ev({
        type: "ListenStatus",
        status: {
          listening: { type: "Ended", host: "9", reason: "Diego left" },
          listeners: [],
        },
      }),
    );
    expect(pc.closed).toBe(true);
    expect(frames.at(-1)).toBeNull();
    expect(useListenStore.getState().countIn).toBe(false);
    expect(useListenStore.getState().listening).toEqual({
      type: "Ended",
      host: "9",
      reason: "Diego left",
    });
  });

  it("falls back to arrival time without rtpTimestamp", () => {
    const { agent, frames, callbacks, advance } = setup();
    agent.onEvent(
      ev({
        type: "ListenStatus",
        status: {
          listening: { type: "Connecting", host: "9", stream: 1 },
          listeners: [],
        },
      }),
    );
    agent.onEvent(ev({ type: "StreamClock", from: "9", stream: 1, clock: clock(0, 8) }));
    advance(250);
    callbacks.shift()!();
    expect(frames.at(-1)!.transport.position).toBeCloseTo(8.5, 9);
  });

  it("replaces the receiver when the stream changes and reports receiver failures", async () => {
    const { agent, sent } = setup();
    agent.onEvent(
      ev({
        type: "ListenStatus",
        status: {
          listening: { type: "Connecting", host: "9", stream: 1 },
          listeners: [],
        },
      }),
    );
    const first = FakePeerConnection.last!;
    agent.onEvent(
      ev({
        type: "ListenStatus",
        status: {
          listening: { type: "Connecting", host: "3", stream: 2 },
          listeners: [],
        },
      }),
    );
    expect(first.closed).toBe(true);
    const second = FakePeerConnection.last!;
    expect(second).not.toBe(first);
    second.setState("failed");
    expect(sent.at(-1)).toEqual({
      type: "SendSignal",
      to: "3",
      stream: 2,
      signal: { type: "Bye", reason: "the connection failed" },
    });
    expect(useListenStore.getState().error).toBe("the connection failed");
    agent.dispose();
  });
});
