/** MockTransport: `MediaRef` (v0.2, media-references). */
import { describe, expect, it } from "vitest";
import type { MediaRef, MediaSource } from "@/generated";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { LIBRARY_ID } from "../library";
import { libraryPath } from "./mediaReferences";
import { project, undo, useMock } from "./testUtils";

const lib = (path: string): MediaSource => ({ type: "Location", location: { type: "Library", id: LIBRARY_ID }, path });

describe("MockTransport MediaRef (media-references)", () => {
  const f = useMock();

  async function importLib(path: string): Promise<MediaRef> {
    const reply = await f.mock.send(cmd("Media", { type: "Import", id: newId(), source: lib(path) }));
    if (reply.type !== "Media") throw new Error(reply.type);
    return reply.media;
  }

  /** Save, then reopen the current project (every media is checked again). */
  async function reopen() {
    const id = project(f).id;
    await f.mock.send(cmd("Project", { type: "Save" }));
    f.events.length = 0;
    await f.mock.send(cmd("Project", { type: "Open", id }));
  }

  const missingEvents = () => f.events.filter((e) => e.type === "Media" && e.event.type === "Missing");

  it("references library files in place", async () => {
    const m = await importLib("Drums/Kick.wav");
    expect(m.location).toEqual({ type: "External", path: libraryPath("Drums/Kick.wav") });
    expect(await f.mock.send(cmd("MediaRef", { type: "ListMissing" }))).toEqual({ type: "MissingMedia", media: [] });
  });

  it("reports missing samples on open and relinks them (one undo step)", async () => {
    const m = await importLib("Drums/Kick.wav");
    f.mock.mediaRefs.setOffline(libraryPath("Drums/Kick.wav"));
    await reopen();
    expect(missingEvents()).toEqual([{ type: "Media", event: { type: "Missing", media: m.id } }]);
    expect(await f.mock.send(cmd("MediaRef", { type: "ListMissing" }))).toEqual({ type: "MissingMedia", media: [m.id] });

    // Another file with the same format: relinked, with a warning (different content).
    await f.mock.send(cmd("MediaRef", { type: "Relink", media: m.id, source: lib("Drums/Snare.wav") }));
    expect(project(f).media[m.id]!.location).toEqual({ type: "External", path: libraryPath("Drums/Snare.wav") });
    expect(f.events).toContainEqual({ type: "MediaRef", event: { type: "Resolved", media: m.id } });
    expect(f.events.some((e) => e.type === "Notification" && e.level === "Warning")).toBe(true);
    // Another format: refused.
    await expect(f.mock.send(cmd("MediaRef", { type: "Relink", media: m.id, source: lib("Drums/Loops/Break 120.wav") }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    // Undo: missing again.
    await undo(f);
    expect(project(f).media[m.id]!.location).toEqual({ type: "External", path: libraryPath("Drums/Kick.wav") });
    expect(missingEvents()).toHaveLength(2);
    // OS paths need the desktop engine.
    await expect(f.mock.send(cmd("MediaRef", { type: "Relink", media: m.id, source: { type: "Path", path: "/x.wav" } }))).rejects.toMatchObject({
      code: "Unsupported",
    });
  });

  it("searches the library: same content relinked, same name offered", async () => {
    const kick = await importLib("Drums/Kick.wav");
    const snare = await importLib("Drums/Snare.wav");
    f.mock.mediaRefs.setOffline(libraryPath("Drums/Snare.wav"));
    // The kick's file is "moved": its record points elsewhere.
    await f.mock.send(cmd("MediaRef", { type: "Relink", media: kick.id, source: lib("Drums/Kick.wav") }));
    await reopen();
    expect(missingEvents().map((e) => e.type === "Media" && e.event.type === "Missing" && e.event.media)).toEqual([snare.id]);
    await f.mock.send(cmd("MediaRef", { type: "Search", media: null }));
    expect(f.events).toContainEqual({ type: "MediaRef", event: { type: "Candidates", media: snare.id, candidates: [] } });
    const done = f.events.filter((e) => e.type === "MediaRef" && e.event.type === "SearchProgress").at(-1);
    expect(done).toMatchObject({ event: { total: expect.any(Number) } });
    await expect(f.mock.send(cmd("MediaRef", { type: "Search", media: null, folder: "/tmp" }))).rejects.toMatchObject({ code: "Unsupported" });
  });

  it("collects external samples into the project in one undo step, then saves", async () => {
    const kick = await importLib("Drums/Kick.wav");
    const gone = await importLib("Drums/Snare.wav");
    f.mock.mediaRefs.setOffline(libraryPath("Drums/Snare.wav"));
    await f.mock.send(cmd("MediaRef", { type: "CollectAll" }));
    expect(project(f).media[kick.id]!.location).toEqual({ type: "Project" });
    expect(project(f).media[gone.id]!.location.type).toBe("External");
    expect(f.events).toContainEqual({ type: "MediaRef", event: { type: "CollectProgress", done: 2, total: 2 } });
    expect(f.events.some((e) => e.type === "Project" && e.event.type === "Saved")).toBe(true);
    // Collected media survive their original going away.
    f.mock.mediaRefs.setOffline(libraryPath("Drums/Kick.wav"));
    await reopen();
    expect(missingEvents().map((e) => e.type === "Media" && e.event.type === "Missing" && e.event.media)).toEqual([gone.id]);
  });
});
