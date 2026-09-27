import { describe, expect, it } from "vitest";
import type { Clip, Note } from "@/generated";
import { TempoMap, type GridStep } from "@/timeline";
import { clipSongStart, clipTempoMap, contentEnd, contentToSong, songToContent } from "./clipTime";
import { isBlackKey, noteHitZone, pitchName, pitchToY, yToPitch } from "./geometry";
import { clampMove, MIN_NOTE_BEATS, MIN_VELOCITY, moveEdits, newNote, nudgeEdits, quantizeCommand, resizeEdits, velocityEdits } from "./noteEdits";

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

  it("clamps new notes", () => {
    expect(newNote("n", 200, -1, 0)).toMatchObject({ pitch: 127, start: 0, duration: MIN_NOTE_BEATS });
  });

  it("quantizes the selection, or the whole clip when nothing is selected", () => {
    expect(quantizeCommand("c", ["a"], 0.25)).toEqual({
      domain: "Note",
      command: { type: "Quantize", clip: "c", notes: ["a"], grid: 0.25, strength: 1, ends: false, swing: 0 },
    });
    expect(quantizeCommand("c", [], 0.5).command).toMatchObject({ notes: null, grid: 0.5 });
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
