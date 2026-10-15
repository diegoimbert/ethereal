/** Mock rules aligned with the controller (Rust is the source of truth). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { createTrack, project, testId, useMock } from "./testUtils";
import { fileStem, takeChars } from "./shared";

describe("mock alignment with the controller", () => {
  const f = useMock();

  it("treats creating an existing id as a no-op (doc/tracks.rs, clips.rs, ...)", async () => {
    const t = await createTrack(f, "Midi");
    const before = project(f);
    await f.mock.send(cmd("Track", { type: "Create", id: t, kind: "Audio", name: "again", color: null, parent: null, before: null }));
    expect(project(f).tracks[t]).toEqual(before.tracks[t]);
    const clip = testId();
    const create = cmd("Clip", { type: "CreateMidi", id: clip, track: t, start: 0, length: 4, name: null });
    await f.mock.send(create);
    await f.mock.send(create);
    expect(Object.values(project(f).clips).filter((c) => c.id === clip)).toHaveLength(1);
  });

  it("keeps existing notes when they are added again (doc/notes.rs)", async () => {
    const t = await createTrack(f, "Midi");
    const clip = testId();
    await f.mock.send(cmd("Clip", { type: "CreateMidi", id: clip, track: t, start: 0, length: 4, name: null }));
    const note = { id: testId(), pitch: 60, velocity: 0.5, start: 0, duration: 1 };
    await f.mock.send(cmd("Note", { type: "Add", clip, notes: [note] }));
    await f.mock.send(cmd("Note", { type: "Add", clip, notes: [{ ...note, pitch: 72 }] }));
    expect(project(f).notes[note.id]?.pitch).toBe(60);
  });

  it("caps the count-in at 16 bars (MAX_COUNT_IN_BARS)", async () => {
    await f.mock.send(cmd("Recording", { type: "SetCountIn", bars: 40 }));
    expect(project(f).settings.count_in_bars).toBe(16);
  });

  it("counts characters like Rust's chars() and treats C1 controls as control characters", () => {
    expect(takeChars("🎹🎹🎹", 2)).toBe("🎹🎹");
    expect(fileStem("a\u0085b", "Preset")).toBe("a-b");
    expect(fileStem("  ..hidden. ", "Preset")).toBe("hidden");
  });
});
