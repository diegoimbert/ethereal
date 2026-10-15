import { describe, expect, it } from "vitest";
import type { AudioContent, Clip, Command } from "@/generated";
import { audioSlipSource, coveredContent, isSlipModifier, midiSlipSource, slipCommand, slipPreview, slipRange, slipRate, type SlipClip } from "./slip";

const SR = 48_000;
/** 2 s of audio: 4 beats at 120 BPM. */
const media = { frames: 2 * SR, sample_rate: SR };

function audio(length: number, extra: Partial<Clip> = {}, content: Partial<AudioContent> = {}): Clip {
  return {
    id: "a",
    track: "t",
    start: 8,
    name: "a",
    color: null,
    muted: false,
    length,
    offset: 0,
    looping: { enabled: false, start: 0, end: length },
    content: {
      type: "Audio",
      media: "m1",
      gain: 0,
      transpose: 0,
      fade_in: 0,
      fade_out: 0,
      fade_in_curve: { type: "Linear" },
      fade_out_curve: { type: "Linear" },
      reversed: false,
      warp: { enabled: true, mode: "Complex", source_bpm: 120 },
      ...content,
    },
    ...extra,
  };
}

type Markers = Parameters<typeof audioSlipSource>[2];
const range = (c: Clip, markers: Markers = [], m = media) => slipRange(c, audioSlipSource(c, m, markers, 120, "mock"));
const close = ([lo, hi]: [number, number], want: [number, number]) => {
  expect(lo).toBeCloseTo(want[0], 6);
  expect(hi).toBeCloseTo(want[1], 6);
};

describe("slipRange (audio)", () => {
  it("has no room when the clip covers the whole file", () => {
    close(range(audio(4)), [0, 0]);
  });

  it("gives a short clip the rest of the file", () => {
    close(range(audio(1)), [0, 3]);
    close(range(audio(1, { offset: 1 })), [-1, 2]);
  });

  it("measures the source with the clip's transpose (Repitch plays faster)", () => {
    // Unwarped, an octave up: one beat of clip reads two beats of source.
    const c = audio(1, {}, { transpose: 12, warp: { enabled: false, mode: "Repitch", source_bpm: null } });
    close(range(c), [0, 2]);
    expect(slipRate(c, [], "mock")).toBeCloseTo(2, 9);
    // Complex keeps the timing: transpose doesn't change the room.
    const complex = audio(1, {}, { transpose: 12 });
    close(range(complex), [0, 3]);
    expect(slipRate(complex, [], "mock")).toBe(1);
  });

  it("maps through the warp markers", () => {
    // 4 content beats per source second: the 2 s file is 8 content beats long.
    const markers = [
      { beat: 0, source: 0 },
      { beat: 4, source: 1 },
    ];
    close(range(audio(2), markers), [0, 6]);
  });

  it("treats a reversed clip like a forward one (it plays the reversed file)", () => {
    close(range(audio(1, {}, { reversed: true })), [0, 3]);
  });

  it("keeps a looped clip's loop region inside the source", () => {
    // Loop [1, 2) repeated over 8 beats: the loop can move by -1 .. +2.
    close(range(audio(8, { offset: 1, looping: { enabled: true, start: 1, end: 2 } })), [-1, 2]);
    // An intro before the loop: content 0 .. loop end 3 is played.
    close(range(audio(8, { offset: 0, looping: { enabled: true, start: 1, end: 3 } })), [0, 1]);
  });

  it("never jumps: a leading gap only shrinks, a source shorter than the clip has no room", () => {
    close(range(audio(1, { offset: -2 })), [0, 5]);
    close(range(audio(6)), [0, 0]);
  });

  it("is unbounded upward without a source (missing media)", () => {
    expect(slipRange(audio(1), null)).toEqual([0, Infinity]);
  });
});

describe("slipRange (MIDI)", () => {
  it("is bounded by the last note's end", () => {
    const c: Clip = { ...audio(4), content: { type: "Midi" } };
    close(slipRange(c, midiSlipSource(c, [{ start: 6, duration: 2 }])), [0, 4]);
    close(slipRange(c, midiSlipSource(c, [{ start: 0, duration: 1 }])), [0, 0]);
  });
});

describe("coveredContent", () => {
  it("is the played content range", () => {
    expect(coveredContent(audio(2, { offset: 1 }))).toEqual([1, 3]);
    expect(coveredContent(audio(8, { offset: 2, looping: { enabled: true, start: 1, end: 3 } }))).toEqual([1, 3]);
  });
});

describe("slipPreview", () => {
  const sc = (c: Clip, r: [number, number], rate = 1): SlipClip => ({ clip: c, range: r, rate });

  it("moves the offset against the pointer, clamped, start and length unchanged", () => {
    const c = audio(1, { offset: 1 });
    const p = (dx: number) => slipPreview([sc(c, [-1, 2])], dx, () => null).get("a")!;
    expect(p(0.5)).toMatchObject({ start: 8, length: 1, offset: 0.5 });
    expect(p(-1.25).offset).toBeCloseTo(2.25, 9);
    expect(p(5).offset).toBe(0);
    expect(p(-5).offset).toBe(3);
  });

  it("scales by the rate (transposed Repitch)", () => {
    const c = audio(1, { offset: 1 });
    expect(slipPreview([sc(c, [-1, 2], 2)], 0.25, () => null).get("a")!.offset).toBeCloseTo(0.5, 9);
  });

  it("snaps the offset to the grid step", () => {
    const c = audio(1, { offset: 1 });
    const p = (dx: number) => slipPreview([sc(c, [-1, 2])], dx, () => 0.5).get("a")!.offset;
    expect(p(-0.6)).toBe(1.5);
    expect(p(0.3)).toBe(0.5);
    expect(p(0.1)).toBe(1);
  });

  it("moves a looped clip's loop region with the offset", () => {
    const c = audio(8, { offset: 1, looping: { enabled: true, start: 1, end: 2 } });
    expect(slipPreview([sc(c, [-1, 2])], -1, () => null).get("a")).toMatchObject({
      offset: 2,
      looping: { enabled: true, start: 2, end: 3 },
    });
  });

  it("clamps each selected clip on its own", () => {
    const a = audio(1, { id: "a", offset: 0 });
    const b = audio(1, { id: "b", offset: 2 });
    const p = slipPreview([sc(a, [0, 3]), sc(b, [-2, 1])], -2, () => null);
    expect(p.get("a")!.offset).toBe(2);
    expect(p.get("b")!.offset).toBe(3);
  });
});

describe("slipCommand", () => {
  it("is one undo step: SetBounds (same start and length) and SetLoop for looped clips", () => {
    const a = audio(1, { id: "a" });
    const b = audio(8, { id: "b", offset: 1, looping: { enabled: true, start: 1, end: 2 } });
    const p = slipPreview(
      [
        { clip: a, range: [0, 3], rate: 1 },
        { clip: b, range: [-1, 2], rate: 1 },
      ],
      -1,
      () => null,
    );
    const c = slipCommand([a, b], p) as Extract<Command, { domain: "Edit" }>;
    expect(c).toMatchObject({ domain: "Edit", command: { type: "Batch", label: "slip clip content" } });
    expect((c.command as { commands: Command[] }).commands).toEqual([
      { domain: "Clip", command: { type: "SetBounds", id: "a", start: 8, length: 1, offset: 1 } },
      { domain: "Clip", command: { type: "SetBounds", id: "b", start: 8, length: 8, offset: 2 } },
      { domain: "Clip", command: { type: "SetLoop", id: "b", looping: { enabled: true, start: 2, end: 3 } } },
    ]);
  });

  it("is null when nothing moved", () => {
    const a = audio(4);
    expect(
      slipCommand(
        [a],
        slipPreview([{ clip: a, range: [0, 0], rate: 1 }], 3, () => null),
      ),
    ).toBeNull();
  });
});

describe("isSlipModifier", () => {
  it("is Ctrl+Shift or Cmd+Shift", () => {
    expect(isSlipModifier({ shiftKey: true, ctrlKey: true, metaKey: false })).toBe(true);
    expect(isSlipModifier({ shiftKey: true, ctrlKey: false, metaKey: true })).toBe(true);
    expect(isSlipModifier({ shiftKey: true, ctrlKey: false, metaKey: false })).toBe(false);
    expect(isSlipModifier({ shiftKey: false, ctrlKey: true, metaKey: true })).toBe(false);
  });
});
