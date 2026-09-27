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

  it("simulates Listen/StopListening (connecting only: the mock has no media)", () => {
    const events: Event[] = [];
    const c = new MockCollab(stubHost(events));
    expect(() => c.command({ type: "Listen", host: "2" })).toThrow(/session/);
    c.command({ type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Me" });
    expect(() => c.command({ type: "Listen", host: "9" })).toThrow(/peer/);
    c.command({ type: "Listen", host: "2" });
    expect(events.at(-1)).toEqual({
      type: "Collab",
      event: { type: "ListenStatus", status: { listening: { type: "Connecting", host: "2", stream: 1 }, listeners: [] } },
    });
    c.command({ type: "StopListening" });
    expect(events.at(-1)).toMatchObject({ event: { type: "ListenStatus", status: { listening: { type: "Off" } } } });
    const n = events.length;
    c.command({ type: "StopListening" });
    expect(events).toHaveLength(n);
    // The receiver's Bye ends the stream.
    c.command({ type: "Listen", host: "2" });
    c.command({ type: "SendSignal", to: "2", stream: 2, signal: { type: "Bye", reason: "no audio" } });
    expect(events.at(-1)).toMatchObject({ event: { status: { listening: { type: "Ended", host: "2", reason: "no audio" } } } });
  });

  it("stores this site's pointer and simulates a peer's (presence-v2)", () => {
    const events: Event[] = [];
    const c = new MockCollab(stubHost(events));
    const p = { beats: 4, track: "t", y: 0.5 };
    c.simulatePointer("2", p);
    expect(events).toEqual([]); // offline: nothing
    c.command({ type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Me" });
    expect(c.command({ type: "SetPointer", pointer: p })).toEqual({ type: "Unit" });
    expect(c.pointer).toEqual(p);
    c.simulatePointer("2", p);
    expect(events.at(-1)).toEqual({ type: "Collab", event: { type: "Pointer", site: "2", pointer: p } });
    c.command({ type: "Leave" });
    expect(c.pointer).toBeNull();
  });

  it("validates joins like the engine", () => {
    const c = new MockCollab(stubHost([]));
    expect(() => c.command({ type: "Join", server: "http://x", session: "jam", token: null, name: "" })).toThrow(/ws:\/\//);
    expect(() => c.command({ type: "Join", server: "ws://x", session: "a b", token: null, name: "" })).toThrow(/session/);
    expect(validSessionName("jam-1.a_b")).toBe(true);
    expect(validSessionName("")).toBe(false);
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
