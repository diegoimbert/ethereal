/** MockTransport: the agent API (ai-chat's MockAgent). */
import { describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { project, undo, useMock, type MockFixture } from "./testUtils";

const agent = (command: object) => ({ domain: "Agent", command }) as unknown as Command;

interface Result {
  content: string;
  is_error: boolean;
}

async function call(f: MockFixture, name: string, input: unknown, raw?: string): Promise<Result> {
  const r = (await f.mock.send(agent({ type: "CallTool", name, input: raw ?? JSON.stringify(input) }))) as unknown as Result & { type: string };
  expect(r.type).toBe("AgentToolResult");
  return { content: r.content, is_error: r.is_error };
}

async function ok<T = Record<string, unknown>>(f: MockFixture, name: string, input: unknown): Promise<T> {
  const r = await call(f, name, input);
  expect(r.is_error, r.content).toBe(false);
  return JSON.parse(r.content) as T;
}

describe("MockTransport agent API", () => {
  const f = useMock();

  it("ListTools: names, descriptions and JSON-text schemas", async () => {
    const r = (await f.mock.send(agent({ type: "ListTools" }))) as unknown as ReplyValue & {
      tools: Array<{ name: string; description: string; input_schema: string }>;
    };
    expect(r.type).toBe("AgentTools");
    const names = r.tools.map((t) => t.name);
    for (const n of ["get_project_overview", "create_track", "create_midi_clip", "add_notes", "set_track_mix", "set_tempo", "undo", "redo"]) {
      expect(names).toContain(n);
    }
    for (const t of r.tools) {
      expect(t.description.length).toBeGreaterThan(10);
      expect(JSON.parse(t.input_schema)).toMatchObject({ type: "object" });
    }
  });

  it("create a track, a clip and notes: one undo step per call, visible in the document", async () => {
    const { track_id } = await ok<{ track_id: string }>(f, "create_track", { kind: "midi", name: "Bass" });
    const t = project(f).tracks[track_id]!;
    expect(t).toMatchObject({ name: "Bass", kind: "Midi" });
    // A MIDI track gets an instrument, like "New track" in the UI.
    expect(Object.values(project(f).devices).some((d) => d.track === track_id)).toBe(true);

    const { clip_id } = await ok<{ clip_id: string }>(f, "create_midi_clip", { track_id, start_beats: 4, length_beats: 4 });
    expect(project(f).clips[clip_id]).toMatchObject({ track: track_id, start: 4, length: 4 });

    const arp = [60, 63, 67, 72].map((pitch, i) => ({ pitch, start_beats: i * 0.5, duration_beats: 0.5, velocity: 127 }));
    const { note_ids } = await ok<{ note_ids: string[] }>(f, "add_notes", { clip_id, notes: arp });
    expect(note_ids).toHaveLength(4);
    const notes = Object.values(project(f).notes).filter((n) => n.clip === clip_id);
    expect(notes.map((n) => n.pitch).sort()).toEqual([60, 63, 67, 72]);
    expect(notes.every((n) => n.velocity === 1)).toBe(true);

    const read = await ok<{ total: number; notes: Array<{ pitch: number; velocity: number }> }>(f, "get_clip_notes", { clip_id });
    expect(read.total).toBe(4);
    expect(read.notes[0]).toMatchObject({ pitch: 60, velocity: 127 });

    // Overview lists the track with its clip.
    const ov = await ok<{ tracks: Array<{ id: string; clips: Array<{ id: string; notes: number }> }> }>(f, "get_project_overview", {});
    expect(ov.tracks.find((x) => x.id === track_id)?.clips).toEqual([expect.objectContaining({ id: clip_id, notes: 4 })]);

    // Each tool call is ONE undo step.
    await undo(f);
    expect(Object.values(project(f).notes).filter((n) => n.clip === clip_id)).toHaveLength(0);
    expect(project(f).clips[clip_id]).toBeDefined();
    await undo(f);
    expect(project(f).clips[clip_id]).toBeUndefined();
    await undo(f);
    expect(project(f).tracks[track_id]).toBeUndefined();
    expect(Object.values(project(f).devices).some((d) => d.track === track_id)).toBe(false);
  });

  it("set_track_mix and set_tempo change only what's given; the undo tool works", async () => {
    const { track_id } = await ok<{ track_id: string }>(f, "create_track", { kind: "audio" });
    await ok(f, "set_track_mix", { track_id, volume_db: -6, pan: 0.5 });
    expect(project(f).tracks[track_id]!.mixer).toMatchObject({ volume: -6, pan: 0.5, mute: false });
    await ok(f, "set_tempo", { bpm: 92 });
    expect(Object.values(project(f).tempo_points)[0]!.bpm).toBe(92);
    await ok(f, "undo", {});
    expect(Object.values(project(f).tempo_points)[0]!.bpm).not.toBe(92);
  });

  it("errors are tool results with is_error, never command failures", async () => {
    expect((await call(f, "make_coffee", {})).is_error).toBe(true);
    expect(await call(f, "create_track", null, "{not json")).toMatchObject({ is_error: true, content: expect.stringMatching(/JSON/) });
    expect(await call(f, "create_track", { kind: "banjo" })).toMatchObject({ is_error: true, content: expect.stringMatching(/kind/) });
    expect(await call(f, "create_midi_clip", { track_id: "nope", start_beats: 0, length_beats: 4 })).toMatchObject({ is_error: true });
    const { track_id } = await ok<{ track_id: string }>(f, "create_track", { kind: "audio" });
    expect(await call(f, "create_midi_clip", { track_id, start_beats: 0, length_beats: 4 })).toMatchObject({
      is_error: true,
      content: expect.stringMatching(/not a MIDI track/),
    });
    expect(await call(f, "set_track_mix", { track_id, pan: 3 })).toMatchObject({ is_error: true });
    // A failed call leaves the document as it was.
    const before = project(f);
    await call(f, "add_notes", { clip_id: "missing", notes: [{ pitch: 60, start_beats: 0, duration_beats: 1 }] });
    expect(project(f)).toEqual(before);
  });
});
