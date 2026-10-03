/** MockTransport sharing simulation (base-115 stub, docs/SHARING.md). */
import { describe, expect, it } from "vitest";
import type { Event, ShareState } from "@/generated";
import { parseInvite } from "@/domain/invite";
import type { MockHost } from "./host";
import { MockShare } from "./share";

function stubHost(events: Event[]): MockHost {
  return {
    project: () => ({ id: "p1" }) as ReturnType<MockHost["project"]>,
    emit: (e) => events.push(e),
    newId: () => "id",
    applyDocument: () => undefined,
    execute: () => undefined,
  };
}

const lastState = (events: Event[]): ShareState | undefined => {
  const e = events.findLast((x) => x.type === "Share" && x.event.type === "State");
  return e?.type === "Share" && e.event.type === "State" ? e.event.state : undefined;
};

describe("MockShare", () => {
  it("hosts with edit and listen links, participants, reset and stop", () => {
    const events: Event[] = [];
    const s = new MockShare(stubHost(events));
    s.command({ type: "Get" });
    expect(lastState(events)).toEqual({ type: "Off" });
    s.command({ type: "SetIdentity", name: " Diego ", color: null });
    s.command({ type: "Start" });
    const hosting = lastState(events);
    if (hosting?.type !== "Hosting") throw new Error("not hosting");
    expect(hosting.participants).toMatchObject([{ name: "Diego", role: "Host", you: true }]);
    expect(typeof parseInvite(hosting.edit_link ?? "")).toBe("object");
    expect(hosting.listen_link).toMatch(/^https:\/\/etherealws\.pages\.dev\/join\/[\w-]{22}#1L/);

    const member = s.simulateJoin("Ada", "Edit");
    expect(lastState(events)).toMatchObject({ participants: [{ role: "Host" }, { name: "Ada", online: true }] });
    expect(events.at(-1)).toMatchObject({ type: "Share", event: { type: "Notice", notice: { type: "ParticipantJoined", name: "Ada" } } });

    s.command({ type: "ResetLink", role: "Edit" });
    const reset = lastState(events);
    expect(reset?.type === "Hosting" && reset.edit_link !== hosting.edit_link).toBe(true);

    s.command({ type: "SetParticipantRole", member, role: "Listen" });
    expect(lastState(events)).toMatchObject({ participants: [{}, { role: "Listen" }] });
    s.command({ type: "RemoveParticipant", member });
    expect(lastState(events)).toMatchObject({ participants: [{ role: "Host" }] });
    expect(() => s.command({ type: "Leave" })).toThrow(/stops sharing/);
    s.command({ type: "Stop" });
    expect(lastState(events)).toEqual({ type: "Off" });
  });

  it("opens an invite, accepts it, and follows the host going offline", () => {
    const events: Event[] = [];
    const s = new MockShare(stubHost(events));
    s.command({ type: "OpenInvite", link: "https://etherealws.pages.dev/join/AbCdEfGhIjKlMnOpQrStUv#1LbCdEfGhIjKlMnOpQrStUv" });
    expect(lastState(events)).toMatchObject({ type: "Joining", stage: { type: "Ready", invite: { role: "Listen", host: { name: "Mock host" } } } });
    s.command({ type: "AcceptInvite" });
    expect(lastState(events)).toMatchObject({ type: "Joined", role: "Listen", link: { type: "Online" } });
    s.simulateHostOnline(false);
    expect(lastState(events)).toMatchObject({ type: "Joined", link: { type: "HostOffline" } });
    s.command({ type: "Leave" });
    expect(lastState(events)).toEqual({ type: "Off" });

    s.command({ type: "OpenInvite", link: "https://etherealws.pages.dev/song" });
    expect(lastState(events)).toMatchObject({ type: "Joining", stage: { type: "Failed", reason: "BadLink" } });
    s.command({ type: "OpenInvite", link: "ethereal://join/OffdEfGhIjKlMnOpQrStUv#1AbCdEfGhIjKlMnOpQrStUv" });
    expect(lastState(events)).toMatchObject({ type: "Joining", stage: { type: "HostOffline" } });
  });
});
