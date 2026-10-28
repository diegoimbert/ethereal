/** MockTransport: `Freeze` (v0.2). */
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
    expect(
      Object.values(h.project.clips)
        .filter((c) => c.track === t.id)
        .map((c) => c.id),
    ).toEqual([clip]);
    expect(Object.values(h.project.devices).filter((d) => d.track === t.id)).toEqual([]);

    const m = withClips(h.project, "Midi");
    h.send({ type: "Freeze", job: "c", track: m.id, media: newId() });
    h.send({ type: "Cancel", job: "c" });
    expect(h.events.at(-1)).toEqual({ type: "Freeze", event: { type: "Cancelled", job: "c" } });
    h.freeze.step();
    expect(h.project.tracks[m.id]!.freeze).toBeUndefined();
  });

  it("bounces a time selection: one new track per track with clips in the range, one undo step", () => {
    const h = harness();
    const m = withClips(h.project, "Midi");
    const a = withClips(h.project, "Audio");
    const empty = Object.values(h.project.tracks).find((t) => t.kind === "Audio" && !Object.values(h.project.clips).some((c) => c.track === t.id));
    const tracksBefore = Object.keys(h.project.tracks).length;
    const range = (start: number, end: number, tracks = [m.id, a.id]) =>
      h.send({ type: "BounceRange", job: "r", tracks, start, end, seed_tracks: newId(), seed_clips: newId(), seed_media: newId() });
    expect(() => range(9000, 9004)).toThrow(/nothing to bounce/);
    expect(() => range(2, 2)).toThrow(/empty/);
    const clipOf = (t: string) => Object.values(h.project.clips).find((c) => c.track === t && !c.lane)!;
    // Cutting into both tracks' first clips.
    const [cm, ca] = [clipOf(m.id), clipOf(a.id)];
    const start = Math.min(cm.start, ca.start) + 0.25;
    const end = Math.max(cm.start + cm.length, ca.start + ca.length) - 0.25;
    expect(range(start, end, [m.id, a.id, ...(empty ? [empty.id] : [])])).toEqual({ type: "RenderStarted", job: "r" });
    h.freeze.step();
    h.freeze.step();
    const added = Object.values(h.project.tracks).filter((t) => t.name.endsWith(" Bounce"));
    expect(added.map((t) => t.name).sort()).toEqual([`${a.name} Bounce`, `${m.name} Bounce`].sort());
    expect(Object.keys(h.project.tracks)).toHaveLength(tracksBefore + 2);
    for (const t of added) {
      const clips = Object.values(h.project.clips).filter((c) => c.track === t.id);
      expect(clips).toHaveLength(1);
      expect(clips[0]!.start).toBe(start);
    }
    expect(h.project.tracks[m.id]!.mixer.mute && h.project.tracks[a.id]!.mixer.mute).toBe(true);
    expect(h.undo).toHaveLength(1);
    h.undo[0]!();
    expect(Object.keys(h.project.tracks)).toHaveLength(tracksBefore);
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
    // The range plus the tail (the decay of the chain).
    expect(h.project.clips[clip]!.length).toBeGreaterThan(4);
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
    expect(h.send({ type: "Consolidate", job: "c", tracks: [m.id], start: 0, end: 64, seed_clips: newId(), seed_notes: newId(), seed_media: newId() })).toEqual(
      { type: "Unit" },
    );
    const clips = Object.values(h.project.clips).filter((c) => c.track === m.id && !c.lane);
    expect(clips).toHaveLength(1);
    const notesAfter = Object.values(h.project.notes).filter((n) => n.clip === clips[0]!.id).length;
    expect(notesAfter).toBeGreaterThan(0);
    expect(notesAfter).toBeLessThanOrEqual(notesBefore);
  });
});

describe("MockTransport ConvertToSampler", () => {
  it("renders the samples and replaces the instrument, one undo step", () => {
    const h = harness();
    const device = Object.values(h.project.devices).find((d) => !d.pad && !d.chain && h.project.tracks[d.track]?.kind === "Midi")!;
    const t = h.project.tracks[device.track]!;
    const next = newId();
    const mediaBefore = Object.keys(h.project.media).length;
    expect(
      h.send({
        type: "ConvertToSampler",
        job: "s",
        device: device.id,
        new_device: next,
        seed_media: newId(),
        keys: [64, 60, 62],
        velocities: [64, 127],
        note_seconds: 1,
        tail_seconds: 1,
      }),
    ).toEqual({ type: "RenderStarted", job: "s" });
    h.freeze.step();
    h.freeze.step();
    expect(h.project.devices[device.id]).toBeUndefined();
    const sampler = h.project.devices[next]!;
    expect(sampler.name).toBe(`${t.name} Sampler`);
    expect(sampler.params[0]).toBe(0);
    expect(sampler.params[5]).toBe(0);
    if (sampler.kind.type !== "Builtin" || sampler.kind.device.type !== "MultiSampler") throw new Error("not a multisampler");
    expect(sampler.kind.device.zones).toHaveLength(6);
    expect(sampler.kind.device.zones.map((z) => [z.root_key, z.velocities.lo, z.velocities.hi])).toEqual([
      [60, 1, 95],
      [60, 96, 127],
      [62, 1, 95],
      [62, 96, 127],
      [64, 1, 95],
      [64, 96, 127],
    ]);
    expect(Object.keys(h.project.media)).toHaveLength(mediaBefore + 6);
    expect(h.events.at(-1)).toEqual({ type: "Freeze", event: { type: "Done", job: "s" } });
    h.undo.pop()!();
    expect(h.project.devices[device.id]).toEqual(device);
    expect(h.project.devices[next]).toBeUndefined();
  });

  it("converts an instrument nested in a rack in its rack chain", () => {
    const h = harness();
    const synth = Object.values(h.project.devices).find((d) => !d.pad && !d.chain && h.project.tracks[d.track]?.kind === "Midi")!;
    const [rack, chain] = [newId(), newId()];
    const ctx = { tx: new Tx(h.project), newId, position: 0 };
    reduceDocumentCommand(
      ctx,
      cmd("Device", { type: "Insert", id: rack, track: synth.track, device: { type: "Builtin", device: { type: "InstrumentRack" } }, before: null }),
    );
    reduceDocumentCommand(ctx, cmd("Rack", { type: "AddChain", id: chain, rack, name: null, before: null }));
    reduceDocumentCommand(ctx, cmd("Rack", { type: "MoveDevice", id: synth.id, chain, before: null }));
    expect(h.project.devices[synth.id]!.chain).toBe(chain);
    const next = newId();
    h.send({
      type: "ConvertToSampler",
      job: "n",
      device: synth.id,
      new_device: next,
      seed_media: newId(),
      keys: [60],
      velocities: [127],
      note_seconds: 1,
      tail_seconds: 1,
    });
    h.freeze.step();
    h.freeze.step();
    expect(h.project.devices[synth.id]).toBeUndefined();
    expect(h.project.devices[next]!.chain).toBe(chain);
  });
});
