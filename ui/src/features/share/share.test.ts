/** Pure parts of the sharing UI: pill status, toast wording, settings validation. */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { Participant, ShareState } from "@/generated";
import { iceUrlError, nameError, PEER_COLORS, signalUrlError, validIceServers } from "./settings";
import { lastSeenText, othersOnline, sessionStatus } from "./status";
import { noticeToast } from "./toasts";

const person = (name: string, extra: Partial<Participant> = {}): Participant => ({
  member: name,
  site: "1",
  name,
  color: 0xff94a6,
  role: "Edit",
  online: true,
  you: false,
  last_seen_ms: null,
  ...extra,
});
const host = person("Diego", { member: null, role: "Host" });

describe("sessionStatus", () => {
  it("is null when nothing is shared (the Share button shows)", () => {
    expect(sessionStatus({ type: "Off" })).toBeNull();
    expect(sessionStatus({ type: "Joining", stage: { type: "Failed", reason: "BadLink", message: "x" } })).toBeNull();
  });

  it("hosting follows the signaling service", () => {
    const hosting = (signal: Extract<ShareState, { type: "Hosting" }>["signal"]): ShareState => ({
      type: "Hosting",
      project: "p",
      edit_link: null,
      listen_link: null,
      participants: [],
      signal,
    });
    expect(sessionStatus(hosting({ type: "Online" }))).toMatchObject({ tone: "live", label: "Live" });
    expect(sessionStatus(hosting({ type: "Connecting" }))).toMatchObject({ tone: "busy", label: "Connecting…" });
    expect(sessionStatus(hosting({ type: "Offline", reason: "dns" }))).toMatchObject({ tone: "busy", label: "Not joinable", detail: expect.stringMatching(/dns/) });
  });

  it("joined follows the host link and names the host", () => {
    const joined = (link: Extract<ShareState, { type: "Joined" }>["link"]): ShareState => ({
      type: "Joined",
      project: "p",
      role: "Edit",
      participants: [host, person("Ada", { you: true })],
      link,
    });
    expect(sessionStatus(joined({ type: "Online" }))).toMatchObject({ tone: "live", label: "Live" });
    expect(sessionStatus(joined({ type: "Connecting", attempt: 2 }))).toMatchObject({ tone: "busy", label: "Reconnecting…" });
    expect(sessionStatus(joined({ type: "HostOffline", since_ms: 0 }))).toMatchObject({ tone: "idle", label: "Diego offline" });
    expect(sessionStatus({ type: "Joining", stage: { type: "Connecting" } })).toMatchObject({ label: "Connecting…" });
    expect(sessionStatus({ type: "Joining", stage: { type: "HostOffline", local_copy: null } })).toMatchObject({ tone: "idle" });
  });

  it("the pill's avatars are the others online", () => {
    expect(othersOnline([host, person("Me", { you: true }), person("Ada"), person("Tom", { online: false })]).map((p) => p.name)).toEqual([
      "Diego",
      "Ada",
    ]);
  });
});

describe("lastSeenText", () => {
  it("says how long ago an offline member was here", () => {
    const now = Date.UTC(2026, 9, 3, 12);
    expect(lastSeenText(null, now)).toBe("offline");
    expect(lastSeenText(now - 20_000, now)).toBe("last seen just now");
    expect(lastSeenText(now - 5 * 60_000, now)).toBe("last seen 5 min ago");
    expect(lastSeenText(now - 3 * 3_600_000, now)).toBe("last seen 3 h ago");
    expect(lastSeenText(now - 26 * 3_600_000, now)).toBe("last seen yesterday");
    expect(lastSeenText(now - 3 * 86_400_000, now)).toBe("last seen 3 days ago");
    expect(lastSeenText(now - 30 * 86_400_000, now)).toMatch(/^last seen \w+/);
  });
});

describe("noticeToast", () => {
  it("words every notice like §8.7", () => {
    expect(noticeToast({ type: "ParticipantJoined", name: "Ada", color: 0x5cffe8 })).toEqual({ title: "Ada joined", accent: "#5cffe8" });
    expect(noticeToast({ type: "ParticipantLeft", name: "Ada" }).title).toBe("Ada left");
    expect(noticeToast({ type: "HostOffline", host_name: "Diego" })).toEqual({ title: "Diego went offline", text: "You're on an offline copy." });
    expect(noticeToast({ type: "HostBack", host_name: "Diego" }).title).toBe("Diego is back");
    expect(noticeToast({ type: "SharingEnded", host_name: "Diego" })).toMatchObject({ title: expect.stringMatching(/^Diego stopped sharing /), text: "Your copy stays on this computer." });
    expect(noticeToast({ type: "LocalCopyKept", project: "p", name: "Song (local copy)" }).text).toBe("Saved as “Song (local copy)”.");
  });
});

describe("settings validation", () => {
  it("names are 1-64 characters after trimming", () => {
    expect(nameError(" Ada ")).toBeNull();
    expect(nameError("  ")).toBe("Enter a name.");
    expect(nameError("x".repeat(65))).toMatch(/64/);
  });

  it("signaling and ICE server addresses", () => {
    expect(signalUrlError("")).toBeNull();
    expect(signalUrlError("https://signal.example.com/v")).toBeNull();
    expect(signalUrlError("ws://x")).not.toBeNull();
    expect(iceUrlError("stun:stun.example.com:3478")).toBeNull();
    expect(iceUrlError("turn:turn.example.com:3478?transport=udp")).toBeNull();
    expect(iceUrlError("https://x")).not.toBeNull();
    expect(
      validIceServers([
        { urls: ["turn:t:3478"], username: "u", credential: "p" },
        { urls: [""], username: null, credential: null },
      ]),
    ).toEqual([{ urls: ["turn:t:3478"], username: "u", credential: "p" }]);
  });

  it("offers the hub's PEER_COLORS (crates/ether-collab/src/wire.rs)", () => {
    const wire = readFileSync(resolve(__dirname, "../../../../crates/ether-collab/src/wire.rs"), "utf8");
    const block = /PEER_COLORS: \[Color; 8\] = \[([\s\S]*?)\];/.exec(wire)?.[1] ?? "";
    const rust = [...block.matchAll(/Color\(0x([0-9a-f]{6})\)/g)].map((m) => parseInt(m[1]!, 16));
    expect(rust).toHaveLength(8);
    expect([...PEER_COLORS]).toEqual(rust);
  });
});
