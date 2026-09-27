/** MockTransport collab simulation (collab). */
import { describe, expect, it } from "vitest";
import type { Event } from "@/generated";
import { cmd } from "../../cmd";
import { MockCollab, validSessionName } from "./collab";
import type { MockHost } from "./host";
import { useMock } from "./testUtils";

function stubHost(events: Event[]): MockHost {
  return {
    project: () => {
      throw new Error("unused");
    },
    emit: (e) => events.push(e),
    newId: () => "id",
    applyDocument: () => undefined,
    execute: () => undefined,
  };
}

describe("MockCollab", () => {
  it("joins with a simulated peer, re-emits on Get and leaves", () => {
    const events: Event[] = [];
    const c = new MockCollab(stubHost(events));
    c.command({ type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Me" });
    expect(events).toContainEqual({ type: "Collab", event: { type: "Session", status: { type: "Online", session: "jam", site: "1" } } });
    const peers = events.findLast((e) => e.type === "Collab" && e.event.type === "Presence");
    expect(peers).toMatchObject({ event: { peers: [{ name: "Mock peer" }] } });

    c.simulatePeer("3", "Zoe", 0xff94a6, { cursor: 4, selected_tracks: ["t"], selected_clips: [], selected_notes: [], selected_devices: [], view: null });
    expect(events.at(-1)).toMatchObject({ event: { type: "Presence", peers: [{ name: "Mock peer" }, { name: "Zoe" }] } });

    events.length = 0;
    c.command({ type: "Get" });
    expect(events).toHaveLength(2);

    c.command({ type: "Leave" });
    expect(events.at(-2)).toEqual({ type: "Collab", event: { type: "Session", status: { type: "Offline" } } });
    expect(events.at(-1)).toEqual({ type: "Collab", event: { type: "Presence", peers: [] } });
  });

  it("validates joins like the engine", () => {
    const c = new MockCollab(stubHost([]));
    expect(() => c.command({ type: "Join", server: "http://x", session: "jam", token: null, name: "" })).toThrow(/ws:\/\//);
    expect(() => c.command({ type: "Join", server: "ws://x", session: "a b", token: null, name: "" })).toThrow(/session/);
    expect(validSessionName("jam-1.a_b")).toBe(true);
    expect(validSessionName("")).toBe(false);
  });
});

describe("MockCollab hosting (stream-host)", () => {
  const listenStatus = (events: Event[]) =>
    events.findLast((e) => e.type === "Collab" && e.event.type === "ListenStatus") as
      | Extract<Event, { type: "Collab" }>
      | undefined;
  const join = (c: MockCollab) => c.command({ type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Me" });
  const clock = {
    rtp: 1000,
    position: 4,
    playing: true,
    recording: false,
    bpm: 120,
    loop_enabled: false,
    loop_region: { start: 0, end: 8 },
    metronome: false,
    discontinuity: true,
  };

  it("the mock peer listens after SetHosting, with the declared endpoint", () => {
    const events: Event[] = [];
    const c = new MockCollab(stubHost(events));
    // Outside a session: accepted, nothing to host.
    expect(c.command({ type: "SetHosting", allow: true, ui_sender: true, remote_transport: true })).toEqual({ type: "Unit" });
    expect(listenStatus(events)).toBeUndefined();

    join(c);
    c.command({ type: "SetHosting", allow: true, ui_sender: true, remote_transport: true });
    expect(listenStatus(events)?.event).toEqual({
      type: "ListenStatus",
      status: { listening: { type: "Off" }, listeners: [{ site: "2", stream: 1, endpoint: "Ui" }] },
    });
    c.command({ type: "SetHosting", allow: true, ui_sender: false, remote_transport: false });
    expect(c.listeners).toEqual([{ site: "2", stream: 1, endpoint: "Engine" }]);
    expect(c.hosting.remote_transport).toBe(false);

    events.length = 0;
    c.command({ type: "Get" });
    expect(listenStatus(events)).toBeDefined();

    c.command({ type: "SetHosting", allow: false, ui_sender: true, remote_transport: true });
    expect(listenStatus(events)?.event).toMatchObject({ status: { listeners: [] } });
  });

  it("simulates listeners, and a leaving peer stops listening", () => {
    const events: Event[] = [];
    const c = new MockCollab(stubHost(events));
    join(c);
    c.command({ type: "SetHosting", allow: true, ui_sender: true, remote_transport: true });
    c.simulatePeer("3", "Zoe", 0xff94a6, { cursor: null, selected_tracks: [], selected_clips: [], selected_notes: [], selected_devices: [], view: null });
    c.simulateListener("3", 77);
    expect(c.listeners.map((l) => l.site)).toEqual(["2", "3"]);
    c.simulatePeer("3", "Zoe", 0, null);
    expect(c.listeners.map((l) => l.site)).toEqual(["2"]);
    c.simulateListener("2", null);
    expect(listenStatus(events)?.event).toMatchObject({ status: { listeners: [] } });
    c.command({ type: "SetHosting", allow: true, ui_sender: true, remote_transport: true });
    c.command({ type: "Leave" });
    expect(c.listeners).toEqual([]);
  });

  it("accepts signals and stream clocks in a session only", () => {
    const c = new MockCollab(stubHost([]));
    const send = { type: "SendStreamClock", to: "2", stream: 1, clock } as const;
    expect(() => c.command(send)).toThrow(/session/);
    expect(() => c.command({ type: "SendSignal", to: "2", stream: 1, signal: { type: "Bye", reason: null } })).toThrow(/session/);
    join(c);
    expect(c.command(send)).toEqual({ type: "Unit" });
    expect(c.clocks.at(-1)).toEqual({ to: "2", stream: 1, clock });
    expect(() => c.command({ ...send, clock: { ...clock, position: Number.NaN } })).toThrow(/finite/);
    expect(c.command({ type: "SendSignal", to: "2", stream: 1, signal: { type: "Offer", sdp: "v=0" } })).toEqual({ type: "Unit" });
  });
});

describe("MockTransport collab", () => {
  const f = useMock();

  it("simulates a session", async () => {
    await expect(f.mock.send(cmd("Collab", { type: "Get" }))).resolves.toEqual({ type: "Unit" });
    expect(f.events).toContainEqual({ type: "Collab", event: { type: "Session", status: { type: "Offline" } } });
    await f.mock.send(cmd("Collab", { type: "Join", server: "ws://r:1", session: "jam", token: null, name: "Me" }));
    expect(f.events.at(-1)).toMatchObject({ type: "Collab", event: { type: "Presence", peers: [{ name: "Mock peer" }] } });
    await f.mock.send(cmd("Collab", { type: "Leave" }));
    expect(f.events.at(-1)).toEqual({ type: "Collab", event: { type: "Presence", peers: [] } });
  });
});
