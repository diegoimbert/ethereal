/** MockTransport collab simulation (collab). */
import { describe, expect, it } from "vitest";
import type { Event } from "@/generated";
import { cmd } from "../../cmd";
import { MockCollab, collabCommand, validSessionName } from "./collab";
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

describe("MockTransport collab", () => {
  const f = useMock();

  it("accepts collab commands", async () => {
    await expect(f.mock.send(cmd("Collab", { type: "Get" }))).resolves.toEqual({ type: "Unit" });
    await expect(f.mock.send(cmd("Collab", { type: "Leave" }))).resolves.toEqual({ type: "Unit" });
    expect(collabCommand({ type: "Leave" })).toEqual({ type: "Unit" });
  });
});
