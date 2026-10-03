/** MockTransport: `Expression::SetTrackMpe` (v0.3, mpe), same semantics as the Rust controller. */
import { describe, expect, it } from "vitest";
import type { MpeSettings } from "@/generated";
import { DEFAULT_MPE } from "@/features/mpe/model";
import { cmd } from "../../cmd";
import { mpeTakeCommands } from "./mpe";
import { createTrack, project, testId, undo, useMock } from "./testUtils";

describe("MockTransport MPE (mpe)", () => {
  const f = useMock();
  const set = (track: string, mpe: MpeSettings | null) => f.mock.send(cmd("Expression", { type: "SetTrackMpe", track, mpe }));

  it("sets, validates and undoes a MIDI track's MPE settings", async () => {
    const track = await createTrack(f, "Midi");
    const audio = await createTrack(f, "Audio");
    const upper: MpeSettings = { zone: "Upper", member_channels: 7, note_pitch_range: 24, master_pitch_range: 12 };
    await expect(set(audio, upper)).rejects.toMatchObject({ code: "InvalidArgument" });
    for (const bad of [
      { ...upper, member_channels: 0 },
      { ...upper, member_channels: 16 },
      { ...upper, note_pitch_range: 0.5 },
      { ...upper, master_pitch_range: 97 },
    ]) {
      await expect(set(track, bad)).rejects.toMatchObject({ code: "InvalidArgument" });
    }
    await expect(set(testId(), upper)).rejects.toMatchObject({ code: "NotFound" });
    expect(project(f).tracks[track]!.mpe).toBeUndefined();

    await set(track, upper);
    expect(project(f).tracks[track]!.mpe).toEqual(upper);
    await set(track, null);
    expect("mpe" in project(f).tracks[track]!).toBe(false);
    await undo(f);
    expect(project(f).tracks[track]!.mpe).toEqual(upper);
    await undo(f);
    expect(project(f).tracks[track]!.mpe).toBeUndefined();
  });

  it("recording on an MPE track records per-note pitch, pressure and timbre (one undo step)", async () => {
    const keys = Object.values(project(f).tracks).find((t) => t.name === "Keys")!;
    await set(keys.id, DEFAULT_MPE);
    await f.mock.send(cmd("Transport", { type: "Locate", position: 8 }));
    await f.mock.send(cmd("Recording", { type: "Arm", track: keys.id, armed: true, exclusive: true }));
    await f.mock.send(cmd("Recording", { type: "SetRecording", enabled: true }));
    f.mock.tick(16 * 3 * 22); // just over 2 beats: notes on beats 9 and 10
    await f.mock.send(cmd("Recording", { type: "SetRecording", enabled: false }));
    const stopped = f.events.flatMap((e) => (e.type === "Recording" && e.event.type === "Stopped" ? [e.event.clips] : []));
    const clip = project(f).clips[stopped[0]![0]!]!;
    expect(clip.track).toBe(keys.id);
    const notes = Object.values(project(f).notes).filter((n) => n.clip === clip.id);
    expect(notes.length).toBeGreaterThanOrEqual(2);
    for (const n of notes) {
      const kinds = Object.values(project(f).note_expressions)
        .filter((e) => e.note === n.id)
        .map((e) => e.kind)
        .sort();
      expect(kinds).toEqual(["Pitch", "Pressure", "Timbre"]);
    }
    await undo(f);
    expect(Object.values(project(f).note_expressions)).toHaveLength(0);
    expect(project(f).clips[clip.id]).toBeUndefined();
  });

  it("simulated MPE input gives recorded notes per-note curves on MPE tracks only", async () => {
    const track = await createTrack(f, "Midi");
    let n = 0;
    const ids = () => `id-${++n}`;
    expect(mpeTakeCommands(project(f), track, [{ id: "a", duration: 1 }], ids)).toEqual([]);
    await set(track, DEFAULT_MPE);
    const commands = mpeTakeCommands(project(f), track, [{ id: "a", duration: 1 }], ids);
    const kinds = commands.map((c) => (c.domain === "Expression" && c.command.type === "SetNoteExpression" ? c.command.kind : null));
    expect(kinds).toEqual(["Pitch", "Pressure", "Timbre"]);
    // They apply (valid curves).
    const clip = testId();
    const note = testId();
    await f.mock.send(cmd("Clip", { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: null }));
    await f.mock.send(cmd("Note", { type: "Add", clip, notes: [{ id: note, pitch: 60, velocity: 0.8, start: 0, duration: 1 }] }));
    for (const c of mpeTakeCommands(project(f), track, [{ id: note, duration: 1 }], testId)) await f.mock.send(c);
    expect(Object.values(project(f).note_expressions).filter((e) => e.note === note)).toHaveLength(3);
  });
});
