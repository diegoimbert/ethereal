import { describe, expect, it } from "vitest";
import type { Event } from "@/generated";
import { cmd } from "../../cmd";
import { MockTransport } from "../MockTransport";
import { PREVIEW_STEPS } from "./mediaPreview";

describe("mock media preview", () => {
  it("starts, is replaced, stops and finishes with events", async () => {
    const t = new MockTransport({ timers: "manual", seed: 3 });
    const events: Event[] = [];
    t.onEvent((e) => {
      if (e.type === "Media") events.push(e);
    });
    await t.connect();
    const source = { type: "Location", location: { type: "Library", id: "library" }, path: "" } as const;
    const listing = await t.send(cmd("Media", { type: "ListDirectory", location: source.location, path: "" }));
    if (listing.type !== "Directory") throw new Error("no listing");
    const file = listing.listing.entries.find((e) => e.kind === "Audio")
      ?? (await (async () => {
        const dir = listing.listing.entries.find((e) => e.kind === "Directory")!;
        const sub = await t.send(cmd("Media", { type: "ListDirectory", location: source.location, path: dir.path }));
        if (sub.type !== "Directory") throw new Error("no listing");
        return sub.listing.entries.find((e) => e.kind === "Audio")!;
      })());
    const a = { ...source, path: file.path };
    await t.send(cmd("Media", { type: "Preview", source: a }));
    await t.send(cmd("Media", { type: "Preview", source: a }));
    await t.send(cmd("Media", { type: "StopPreview" }));
    await t.send(cmd("Media", { type: "Preview", source: a }));
    t.tick(16 * (PREVIEW_STEPS + 2));
    const kinds = events.map((e) => (e.type === "Media" && e.event.type === "PreviewEnded" ? e.event.reason : e.type === "Media" ? e.event.type : ""));
    expect(kinds).toEqual(["PreviewStarted", "Replaced", "PreviewStarted", "Stopped", "PreviewStarted", "Finished"]);
    t.dispose();
  });
});
