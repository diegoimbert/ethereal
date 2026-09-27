/** MockTransport: `Freeze` (v0.2, freeze-bounce). */
import { describe, expect, it } from "vitest";
import type { Event, Project, ReplyValue, Track } from "@/generated";
import { newId } from "../../ids";
import { createDemoProject } from "../demoProject";
import { reduceDocumentCommand } from "../documentReducer";
import { Tx } from "../tx";
import { MockFreeze, type FreezeHost } from "./freezeBounce";
import { cmd } from "../../cmd";
import { project as snapshot, useMock } from "./testUtils";

/** A `MockFreeze` over a demo project, with the host the MockTransport provides. */
function harness() {
  const project: Project = structuredClone(createDemoProject());
  const events: Event[] = [];
  const undo: Array<() => void> = [];
  const host: FreezeHost = {
    project: () => project,
    emit: (e) => events.push(e),
    newId,
    applyDocument: () => undefined,
    execute: () => undefined,
    transact: (_label, edit) => {
      const tx = new Tx(project);
      const ctx = { tx, newId, position: 0 };
      try {
        edit(tx, (c) => void reduceDocumentCommand(ctx, c));
      } catch (e) {
        tx.rollback();
        throw e;
      }
      const inverse = tx.inverse();
      undo.push(() => {
        const u = new Tx(project);
        for (const c of inverse) u.write(c);
      });
    },
  };
  const freeze = new MockFreeze(host);
  const send = (c: Parameters<MockFreeze["command"]>[0]): ReplyValue => freeze.command(c);
  return { project, events, freeze, send, undo };
}

const withClips = (p: Project, kind: Track["kind"]) =>
  Object.values(p.tracks).find((t) => t.kind === kind && Object.values(p.clips).some((c) => c.track === t.id))!;

describe("MockTransport Freeze (freeze-bounce)", () => {
  const f = useMock();

  it("is routed by the MockTransport (playhead steps, undo)", async () => {
    const p = snapshot(f);
    const t = withClips(p, "Midi");
    const media = newId();
    await expect(f.mock.send(cmd("Freeze", { type: "Freeze", job: "j", track: t.id, media }))).resolves.toEqual({
      type: "RenderStarted",
      job: "j",
    });
    f.mock.tick(16);
    f.mock.tick(16);
    expect(snapshot(f).tracks[t.id]!.freeze).toEqual({ media, start: 0 });
    expect(f.events).toContainEqual({ type: "Freeze", event: { type: "Done", job: "j" } });
    await f.mock.send(cmd("Edit", { type: "Undo" }));
    expect(snapshot(f).tracks[t.id]!.freeze).toBeUndefined();
  });

  it("freezes after two playhead steps, as one undo step", () => {
    const h = harness();
    const t = withClips(h.project, "Midi");
    const media = newId();
    expect(h.send({ type: "Freeze", job: "j", track: t.id, media })).toEqual({ type: "RenderStarted", job: "j" });
    expect(() => h.send({ type: "Freeze", job: "k", track: t.id, media: newId() })).toThrow(/already running/);
    h.freeze.step();
    expect(h.events).toEqual([{ type: "Freeze", event: { type: "Progress", job: "j", progress: 0.5 } }]);
    expect(h.project.tracks[t.id]!.freeze).toBeUndefined();
    h.freeze.step();
    expect(h.project.tracks[t.id]!.freeze).toEqual({ media, start: 0 });
    expect(h.project.media[media]!.file).toMatch(/^media\//);
    expect(h.events.at(-1)).toEqual({ type: "Freeze", event: { type: "Done", job: "j" } });
    h.undo.pop()!();
    expect(h.project.tracks[t.id]!.freeze).toBeUndefined();
    expect(h.project.media[media]).toBeUndefined();
  });

  it("unfreezes, flattens and cancels", () => {
    const h = harness();
    const t = withClips(h.project, "Audio");
    h.send({ type: "Freeze", job: "j", track: t.id, media: newId() });
    h.freeze.step();
    h.freeze.step();
    const clip = newId();
    h.send({ type: "Flatten", track: t.id, clip, new_track: newId() });
    expect(h.project.tracks[t.id]!.freeze).toBeUndefined();
    expect(Object.values(h.project.clips).filter((c) => c.track === t.id).map((c) => c.id)).toEqual([clip]);
    expect(Object.values(h.project.devices).filter((d) => d.track === t.id)).toEqual([]);

    const m = withClips(h.project, "Midi");
    h.send({ type: "Freeze", job: "c", track: m.id, media: newId() });
    h.send({ type: "Cancel", job: "c" });
    expect(h.events.at(-1)).toEqual({ type: "Freeze", event: { type: "Cancelled", job: "c" } });
    h.freeze.step();
    expect(h.project.tracks[m.id]!.freeze).toBeUndefined();
  });

  it("bounces to a new track (source muted) and consolidates MIDI instantly", () => {
    const h = harness();
    const m = withClips(h.project, "Midi");
    const nt = newId();
    const clip = newId();
    h.send({
      type: "Bounce",
      job: "b",
      track: m.id,
      start: 0,
      end: 4,
      include_chain: true,
      media: newId(),
      target: { type: "NewTrack", track: nt, clip },
    });
    h.freeze.step();
    h.freeze.step();
    expect(h.project.tracks[nt]!.name).toBe(`${m.name} Bounce`);
    expect(h.project.clips[clip]!.length).toBeCloseTo(4);
    expect(h.project.tracks[m.id]!.mixer.mute).toBe(true);
    expect(() =>
      h.send({
        type: "Bounce",
        job: "x",
        track: m.id,
        start: 0,
        end: 4,
        include_chain: false,
        media: newId(),
        target: { type: "NewTrack", track: newId(), clip: newId() },
      }),
    ).toThrow(/MIDI/);

    const notesBefore = Object.values(h.project.notes).filter((n) => h.project.clips[n.clip]?.track === m.id).length;
    expect(h.send({ type: "Consolidate", job: "c", tracks: [m.id], start: 0, end: 64, seed_clips: newId(), seed_notes: newId(), seed_media: newId() })).toEqual({ type: "Unit" });
    const clips = Object.values(h.project.clips).filter((c) => c.track === m.id && !c.lane);
    expect(clips).toHaveLength(1);
    const notesAfter = Object.values(h.project.notes).filter((n) => n.clip === clips[0]!.id).length;
    expect(notesAfter).toBeGreaterThan(0);
    expect(notesAfter).toBeLessThanOrEqual(notesBefore);
  });
});
