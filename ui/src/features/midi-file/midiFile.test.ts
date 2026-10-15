import { describe, expect, it } from "vitest";
import type { Command, MidiFileContents, Project, Track } from "@/generated";
import { isMidiName, midiImportCommand } from "./importMidi";
import { midiFileName } from "@/transport/mock/roadmap/midiFile";

const track = (id: string, kind: Track["kind"], order: string, parent: string | null = null) => ({ id, kind, order, parent, name: id }) as Track;

/** Only what `midiImportCommand` reads. */
function project(opts: { clips?: number; tracks?: Track[] } = {}): Project {
  const tracks = opts.tracks ?? [track("keys", "Midi", "a"), track("drums", "Audio", "b"), track("ret", "Return", "c"), track("master", "Master", "d")];
  return {
    tracks: Object.fromEntries(tracks.map((t) => [t.id, t])),
    clips: Object.fromEntries(Array.from({ length: opts.clips ?? 0 }, (_, i) => [`c${i}`, {}])),
    tempo_points: { t0: { id: "t0", time: 0, bpm: 120, curve: "Step" } },
    time_signatures: { s0: { id: "s0", time: 0, signature: { numerator: 4, denominator: 4 } } },
  } as unknown as Project;
}

const file: MidiFileContents = {
  name: "song",
  tracks: [
    { name: "lead", channel: 0, notes: [{ pitch: 60, velocity: 0.5, release_velocity: 0.5, start: 0, duration: 1 }] },
    { name: null, channel: 9, notes: [{ pitch: 36, velocity: 1, release_velocity: 0.5, start: 4, duration: 0.25 }] },
    { name: "empty", channel: 2, notes: [] },
  ],
  tempos: [
    { time: 0, bpm: 90 },
    { time: 6, bpm: 140 },
  ],
  time_signatures: [{ time: 0, signature: { numerator: 3, denominator: 4 } }],
  length: 4.25,
};

function commands(c: Command): Command[] {
  return c.domain === "Edit" && c.command.type === "Batch" ? c.command.commands : [c];
}

const ids = () => {
  let n = 0;
  return () => `id${++n}`;
};

describe("midiImportCommand (#717)", () => {
  it("an empty project: new MIDI tracks per file track with notes, the file's tempo map, bar-long clips; one batch", () => {
    const plan = midiImportCommand(project(), file, { track: null, at: 0 }, ids())!;
    expect(plan.command.domain === "Edit" && plan.command.command.type === "Batch" && plan.command.command.label).toBe("import MIDI file");
    const cs = commands(plan.command);
    const creates = cs.flatMap((c) => (c.domain === "Track" && c.command.type === "Create" ? [c.command] : []));
    // The empty file track is skipped; an unnamed one is named after the file and its channel.
    expect(creates.map((c) => [c.name, c.kind, c.parent, c.before])).toEqual([
      ["lead", "Midi", null, null],
      ["song 10", "Midi", null, null],
    ]);
    expect(plan.tracks).toHaveLength(2);
    // Each new track has the built-in synth.
    expect(cs.filter((c) => c.domain === "Device" && c.command.type === "Insert")).toHaveLength(2);
    expect(cs).toContainEqual({ domain: "Tempo", command: { type: "EditTempoPoint", id: "t0", time: null, bpm: 90, curve: null } });
    expect(cs).toContainEqual(expect.objectContaining({ domain: "Tempo", command: expect.objectContaining({ type: "AddTempoPoint", time: 6, bpm: 140 }) }));
    expect(cs).toContainEqual({ domain: "Tempo", command: { type: "EditTimeSignature", id: "s0", time: null, signature: { numerator: 3, denominator: 4 } } });
    // 4.25 beats in 3/4 → two bars.
    const clips = cs.flatMap((c) => (c.domain === "Clip" && c.command.type === "CreateMidi" ? [c.command] : []));
    expect(clips.map((c) => [c.length, c.start, c.name])).toEqual([
      [6, 0, "lead"],
      [6, 0, "song 10"],
    ]);
    const notes = cs.flatMap((c) => (c.domain === "Note" && c.command.type === "Add" ? c.command.notes : []));
    expect(notes.map((n) => [n.pitch, n.start, n.duration, n.velocity])).toEqual([
      [60, 0, 1, 0.5],
      [36, 4, 0.25, 1],
    ]);
  });

  it("on a MIDI track of a project with clips: the first file track goes there, the others right after it; the tempo is kept", () => {
    const tracks = [track("g", "Group", "a"), track("keys", "Midi", "a", "g"), track("bass", "Midi", "b", "g"), track("master", "Master", "z")];
    const plan = midiImportCommand(project({ clips: 1, tracks }), file, { track: "keys", at: 8 }, ids())!;
    const cs = commands(plan.command);
    expect(cs.some((c) => c.domain === "Tempo")).toBe(false);
    const clips = cs.flatMap((c) => (c.domain === "Clip" && c.command.type === "CreateMidi" ? [c.command] : []));
    expect(clips[0]).toMatchObject({ track: "keys", start: 8, length: 8 });
    const creates = cs.flatMap((c) => (c.domain === "Track" && c.command.type === "Create" ? [c.command] : []));
    expect(creates.map((c) => [c.parent, c.before])).toEqual([["g", "bass"]]);
    expect(clips[1]!.track).toBe(creates[0]!.id);
  });

  it("nothing to import without notes", () => {
    expect(midiImportCommand(project(), { ...file, tracks: [file.tracks[2]!] }, { track: null, at: 0 })).toBeNull();
  });

  it("recognizes MIDI file names; the mock names files like the controller", () => {
    expect(["a.mid", "B.MIDI", "x.y.Mid"].every(isMidiName)).toBe(true);
    expect(["a.wav", "mid", "a.mid.wav"].some(isMidiName)).toBe(false);
    expect(midiFileName("my: riff")).toBe("my_ riff.mid");
    expect(midiFileName(" .. ")).toBe("midi.mid");
  });
});
