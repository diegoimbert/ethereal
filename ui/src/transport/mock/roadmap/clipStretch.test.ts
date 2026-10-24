/** MockTransport: `Clip::Stretch` (the controller's twin: crates/ether-controller/tests/it/clip_stretch.rs). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { project, trackNamed, undo, useMock } from "./testUtils";

describe("MockTransport Clip::Stretch", () => {
  const f = useMock();

  it("MIDI: the notes scale with the clip; one undo step", async () => {
    const clip = Object.values(project(f).clips).find((c) => c.content.type === "Midi" && Object.values(project(f).notes).some((n) => n.clip === c.id))!;
    const notes = Object.values(project(f).notes).filter((n) => n.clip === clip.id);
    await f.mock.send(cmd("Clip", { type: "Stretch", id: clip.id, start: clip.start, length: clip.length * 2 }));
    expect(project(f).clips[clip.id]).toMatchObject({ start: clip.start, length: clip.length * 2, offset: clip.offset * 2 });
    for (const n of notes) expect(project(f).notes[n.id]).toMatchObject({ start: n.start * 2, duration: n.duration * 2 });
    await undo(f);
    expect(project(f).clips[clip.id]).toEqual(clip);
  });

  it("audio: an unwarped clip gets warped, its markers keep the audio under the clip", async () => {
    const clip = Object.values(project(f).clips).find((c) => c.track === trackNamed(f, "Drums").id)!;
    await f.mock.send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { enabled: false, mode: "Repitch", source_bpm: null } }));
    const before = project(f).clips[clip.id]!;
    await f.mock.send(cmd("Clip", { type: "Stretch", id: clip.id, start: before.start, length: before.length / 2 }));
    const after = project(f).clips[clip.id]!;
    expect(after.content).toMatchObject({ warp: { enabled: true, mode: "Repitch" } });
    const markers = Object.values(project(f).warp_markers).filter((m) => m.clip === clip.id);
    expect(markers).toHaveLength(2);
    expect(Math.min(...markers.map((m) => m.beat))).toBeCloseTo(Math.max(0, before.offset) / 2, 9);
    await expect(f.mock.send(cmd("Clip", { type: "Stretch", id: clip.id, start: 0, length: 0 }))).rejects.toThrow();
  });
});
