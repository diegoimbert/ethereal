import { describe, expect, it } from "vitest";
import type { Clip, Note } from "@/generated";
import { TempoMap, type GridStep } from "@/timeline";
import { clipSongStart, clipTempoMap, contentEnd, contentFloor, contentToSong, songToContent } from "./clipTime";
import { singleClipSpace } from "./rollSpace";
import { isBlackKey, noteHitZone, pitchName, pitchToY, yToPitch } from "./geometry";
import {
  clampMove,
  midiVelocity,
  MIN_NOTE_BEATS,
  MIN_VELOCITY,
  moveEdits,
  newNote,
  nudgeEdits,
  resizeEdits,
  velocityDrag,
  VELOCITY_DRAG_PX,
  VELOCITY_FINE,
  velocityEdits,
} from "./noteEdits";

const tempo = TempoMap.constant(120);
const sixteenth: GridStep = { kind: "beats", beats: 0.25 };

const note = (id: string, pitch: number, start: number, duration: number, velocity = 0.8): Note => ({
  id,
  clip: "c",
  pitch,
  velocity,
  release_velocity: 0.5,
  start,
  duration,
  muted: false,
});

const clip = (over: Partial<Clip> = {}): Clip => ({
  id: "c",
  track: "t",
  start: 8,
  name: "Clip",
  color: null,
  muted: false,
  length: 8,
  offset: 0,
  looping: { enabled: false, start: 0, end: 4 },
  content: { type: "Midi" },
  ...over,
});

describe("moveEdits", () => {
  it("snaps the anchor to the grid and moves the others by the same delta", () => {
    const a = note("a", 60, 1, 1);
    const b = note("b", 64, 2.1, 1);
    const edits = moveEdits([a, b], a, 0.9, 2, sixteenth, tempo);
    expect(edits.map((e) => [e.start, e.pitch])).toEqual([
      [2, 62],
      [expect.closeTo(3.1, 9), 66],
    ]);
    expect(edits[0]!.duration).toBeNull();
    expect(edits[0]!.velocity).toBeNull();
  });

  it("keeps an off-grid anchor's offset within the grid (relative snap)", () => {
    const beat: GridStep = { kind: "beats", beats: 1 };
    const a = note("a", 60, 4.34, 1);
    const b = note("b", 62, 6, 1);
    expect(moveEdits([a, b], a, 0.8, 0, beat, tempo).map((e) => e.start)).toEqual([expect.closeTo(5.34, 9), expect.closeTo(7, 9)]);
    expect(moveEdits([a], a, -1.3, 0, beat, tempo)[0]!.start).toBeCloseTo(3.34);
    // Clamped at 0: the selection stops at the start of the clip.
    expect(moveEdits([a], a, -9, 0, beat, tempo)[0]!.start).toBe(0);
  });

  it("does not snap with a null step", () => {
    const a = note("a", 60, 1, 1);
    expect(moveEdits([a], a, 0.33, 0, null, tempo)[0]!.start).toBeCloseTo(1.33);
  });

  it("clamps so no note goes before 0 or outside 0..127", () => {
    const a = note("a", 120, 1, 1);
    const b = note("b", 10, 3, 1);
    expect(clampMove([a, b], -5, 20)).toEqual({ dBeats: -1, dPitch: 7 });
    expect(clampMove([a, b], 0, -20)).toEqual({ dBeats: 0, dPitch: -10 });
    const edits = moveEdits([a, b], b, -10, 20, sixteenth, tempo);
    expect(edits.map((e) => e.start)).toEqual([0, 2]);
    expect(edits.map((e) => e.pitch)).toEqual([127, 17]);
  });

  it("nudges by exact deltas", () => {
    const a = note("a", 60, 1, 1);
    expect(nudgeEdits([a], 0.25, 12)[0]).toMatchObject({ start: 1.25, pitch: 72 });
  });
});

describe("resizeEdits", () => {
  it("snaps the anchor's end and applies the delta to every note", () => {
    const a = note("a", 60, 0, 1);
    const b = note("b", 62, 0, 0.5);
    const edits = resizeEdits([a, b], a, "end", 0.6, sixteenth, tempo);
    expect(edits.map((e) => e.duration)).toEqual([1.5, 1]);
    expect(edits[0]!.start).toBeNull();
  });

  it("never goes below the minimum length", () => {
    const a = note("a", 60, 0, 1);
    expect(resizeEdits([a], a, "end", -5, null, tempo)[0]!.duration).toBe(MIN_NOTE_BEATS);
  });

  it("moves the start edge keeping the end fixed", () => {
    const a = note("a", 60, 1, 2);
    const e = resizeEdits([a], a, "start", 0.55, sixteenth, tempo)[0]!;
    expect(e.start).toBe(1.5);
    expect(e.duration).toBe(1.5);
    const clamped = resizeEdits([a], a, "start", 10, null, tempo)[0]!;
    expect(clamped.start! + clamped.duration!).toBeCloseTo(3);
    expect(clamped.duration).toBeCloseTo(MIN_NOTE_BEATS);
    expect(resizeEdits([a], a, "start", -5, null, tempo)[0]).toMatchObject({ start: 0, duration: 3 });
  });
});

describe("velocity, new notes, quantize", () => {
  it("offsets velocities and clamps them", () => {
    const edits = velocityEdits([note("a", 60, 0, 1, 0.5), note("b", 60, 0, 1, 0.9)], 0.3);
    expect(edits.map((e) => e.velocity)).toEqual([0.8, 1]);
    expect(velocityEdits([note("a", 60, 0, 1, 0.1)], -1)[0]!.velocity).toBe(MIN_VELOCITY);
  });

  it("alt-drag velocity: up is louder, the full range over VELOCITY_DRAG_PX", () => {
    const drag = velocityDrag();
    expect(drag(-VELOCITY_DRAG_PX / 2, false)).toBeCloseTo(0.5);
    expect(drag(-VELOCITY_DRAG_PX, false)).toBeCloseTo(1);
    expect(drag(0, false)).toBeCloseTo(0);
    expect(velocityDrag()(VELOCITY_DRAG_PX / 4, false)).toBeCloseTo(-0.25);
  });

  it("alt-drag velocity: shift is fine and toggling it mid-drag never jumps", () => {
    const drag = velocityDrag();
    expect(drag(-90, false)).toBeCloseTo(0.5);
    // Shift pressed at the same pointer position: unchanged.
    expect(drag(-90, true)).toBeCloseTo(0.5);
    // 90 px more with shift: a tenth of the coarse amount.
    expect(drag(-180, true)).toBeCloseTo(0.5 + 0.5 * VELOCITY_FINE);
    expect(drag(-180, false)).toBeCloseTo(0.5 + 0.5 * VELOCITY_FINE);
  });

  it("alt-drag velocity: the same delta on every note, clamped like the lane", () => {
    const dv = velocityDrag()(-VELOCITY_DRAG_PX * 0.3, false);
    const edits = velocityEdits([note("a", 60, 0, 1, 0.5), note("b", 62, 1, 1, 0.9)], dv);
    expect(edits[0]!.velocity).toBeCloseTo(0.8);
    expect(edits[1]!.velocity).toBe(1);
    // Velocity only: no time or pitch change.
    expect(edits.every((e) => e.start === null && e.pitch === null && e.duration === null)).toBe(true);
  });

  it("shows velocity as MIDI 1..127", () => {
    expect(midiVelocity(1)).toBe(127);
    expect(midiVelocity(100 / 127)).toBe(100);
    expect(midiVelocity(MIN_VELOCITY)).toBe(1);
  });

  it("clamps new notes", () => {
    expect(newNote("n", 200, -1, 0)).toMatchObject({ pitch: 127, start: 0, duration: MIN_NOTE_BEATS });
  });
});

describe("clip time", () => {
  it("reads the clip's song position through one accessor", () => {
    expect(clipSongStart(clip())).toBe(8);
  });

  it("maps song time to content time without looping", () => {
    const c = clip({ offset: 2 });
    expect(songToContent(c, 7)).toBeNull();
    expect(songToContent(c, 8)).toBe(2);
    expect(songToContent(c, 11)).toBe(5);
    expect(songToContent(c, 16)).toBeNull();
    expect(contentEnd(c)).toBe(10);
  });

  it("wraps into the loop region when looping", () => {
    const c = clip({ offset: 1, looping: { enabled: true, start: 0, end: 4 } });
    expect(songToContent(c, 8)).toBe(1);
    expect(songToContent(c, 10.5)).toBe(3.5);
    expect(songToContent(c, 11)).toBe(0);
    expect(songToContent(c, 13.5)).toBe(2.5);
    expect(contentEnd(c)).toBe(4);
  });

  it("maps content time back to the first song position that plays it", () => {
    const c = clip({ offset: 1, looping: { enabled: true, start: 0, end: 4 } });
    expect(contentToSong(c, 2)).toBe(9);
    expect(contentToSong(c, 0.5)).toBe(11.5);
    expect(contentToSong(clip(), 100)).toBe(16);
  });

  it("uses the tempo and signature at the clip start, with bar 1 at content 0", () => {
    const song = new TempoMap(
      [
        { id: "a", time: 0, bpm: 120, curve: "Step" },
        { id: "b", time: 4, bpm: 90, curve: "Step" },
      ],
      [
        { id: "s", time: 0, signature: { numerator: 4, denominator: 4 } },
        { id: "s2", time: 8, signature: { numerator: 3, denominator: 4 } },
      ],
    );
    const t = clipTempoMap(song, clip());
    expect(t.bpmAt(0)).toBe(90);
    expect(t.signatureAt(0)).toEqual({ numerator: 3, denominator: 4 });
    expect(t.barToBeats(2)).toBe(3);
  });
});

describe("geometry", () => {
  it("maps pitches to rows (127 on top)", () => {
    expect(pitchToY(127, 10)).toBe(0);
    expect(pitchToY(0, 10)).toBe(1270);
    expect(yToPitch(5, 10)).toBe(127);
    expect(yToPitch(1275, 10)).toBe(0);
    expect(yToPitch(-50, 10)).toBe(127);
  });

  it("names keys", () => {
    expect(pitchName(60)).toBe("C3");
    expect(pitchName(0)).toBe("C-2");
    expect(isBlackKey(61)).toBe(true);
    expect(isBlackKey(64)).toBe(false);
  });

  it("finds resize zones", () => {
    expect(noteHitZone(1, 100)).toBe("start");
    expect(noteHitZone(50, 100)).toBe("body");
    expect(noteHitZone(98, 100)).toBe("end");
    expect(noteHitZone(4, 12)).toBe("body");
  });
});

describe("a clip extended to the left past its content start (negative offset)", () => {
  // Song 6..16 playing content -2..8: the editor's axis starts at content -2.
  const extended = clip({ start: 6, length: 10, offset: -2 });

  it("the axis, placement and move limits start at the clip start, not at content 0", () => {
    expect(contentFloor(extended)).toBe(-2);
    expect(contentFloor(clip({ offset: 1 }))).toBe(0);
    const space = singleClipSpace(extended);
    expect(space.floor).toBe(-2);
    expect(space.place(-1.5)).toEqual({ clip: extended, content: -1.5 });
    expect(space.place(-5)).toEqual({ clip: extended, content: -2 });
    // Song ↔ content: the gap plays content -2..0 at song 6..8.
    expect(space.fromSong(7)).toBe(-1);
    expect(space.toSong(-1)).toBe(7);
    // A note moves into the gap, down to the clip start.
    const n = note("n", 60, 1, 1);
    expect(moveEdits([n], n, -2.5, 0, sixteenth, tempo, space.limits)[0]!.start).toBe(-1.5);
    expect(moveEdits([n], n, -9, 0, sixteenth, tempo, space.limits)[0]!.start).toBe(-2);
    expect(nudgeEdits([n], -9, 0, space.limits)[0]!.start).toBe(-2);
  });

  it("notes in the gap keep their place: moves, resizes and new notes never pull them to 0", () => {
    const g = note("g", 60, -1, 0.5);
    // Without limits (from 0 on) a note before 0 is not pushed forward, only kept from going further back.
    expect(clampMove([g], 0.25, 0).dBeats).toBe(0.25);
    expect(clampMove([g], -1, 0).dBeats).toBe(0);
    expect(nudgeEdits([g], 0.25, 0)[0]!.start).toBe(-0.75);
    expect(nudgeEdits([g], 0, 1)[0]).toMatchObject({ start: -1, pitch: 61 });
    // Start-edge resize: down to the floor.
    expect(resizeEdits([g], g, "start", -0.5, sixteenth, tempo, -2)[0]).toMatchObject({ start: -1.5, duration: 1 });
    expect(resizeEdits([g], g, "start", -5, sixteenth, tempo, -2)[0]).toMatchObject({ start: -2, duration: 1.5 });
    expect(newNote("x", 60, -1.25, 1, 0.8, -2).start).toBe(-1.25);
    expect(newNote("x", 60, -3, 1, 0.8, -2).start).toBe(-2);
    expect(newNote("x", 60, -1, 1).start).toBe(0);
  });
});
