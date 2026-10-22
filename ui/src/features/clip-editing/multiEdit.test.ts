import { describe, expect, it } from "vitest";
import type { AudioContent, Clip, Command, WarpFormants } from "@/generated";
import {
  audioClips,
  common,
  FORMANTS_DEFAULT,
  formantBases,
  formantsOf,
  gainBases,
  offsetFormant,
  offsetGain,
  offsetTranspose,
  offsetValue,
  setAllFormant,
  setAllTranspose,
  transposeBases,
  type AudioClip,
} from "./multiEdit";

function clip(id: string, audio: Partial<AudioContent> = {}): Clip {
  const content: AudioContent = {
    media: "M",
    gain: 0,
    transpose: 0,
    fade_in: 0,
    fade_out: 0,
    fade_in_curve: { type: "Linear" },
    fade_out_curve: { type: "Linear" },
    reversed: false,
    warp: { enabled: false, mode: "Repitch", source_bpm: null },
    ...audio,
  };
  return {
    id,
    track: "T",
    start: 0,
    name: "",
    color: null,
    muted: false,
    length: 4,
    offset: 0,
    looping: { enabled: false, start: 0, end: 4 },
    content: { type: "Audio", ...content },
  } as Clip;
}

/** The (clip id, value) pairs a builder's command sets. */
function sets(c: Command | null): Array<[string, number]> {
  if (!c) return [];
  if (c.domain === "Edit" && c.command.type === "Batch") return c.command.commands.flatMap((x) => sets(x));
  if (c.domain === "Clip" && c.command.type === "SetTranspose") return [[c.command.id, c.command.semitones]];
  if (c.domain === "Clip" && c.command.type === "SetGain") return [[c.command.id, c.command.gain]];
  return [];
}

describe("offsetValue (relative multi-clip edits)", () => {
  it("adds the drag's offset to each clip's own value", () => {
    expect(offsetValue(-6, 2, -48, 48)).toBe(-4);
    expect(offsetValue(3, 2, -48, 48)).toBe(5);
    expect(offsetValue(0, -0.5, -60, 24)).toBe(-0.5);
  });
  it("clamps each clip to the range on its own", () => {
    expect(offsetValue(47, 2, -48, 48)).toBe(48);
    expect(offsetValue(-47, -5, -48, 48)).toBe(-48);
    expect(offsetValue(20, 10, -60, 24)).toBe(24);
  });
  it("keeps float steps exact", () => {
    expect(offsetValue(0.1, 0.2, -60, 24)).toBe(0.3);
  });
});

describe("multi audio-clip builders", () => {
  const list = (): AudioClip[] => audioClips([clip("a", { transpose: -6 }), clip("b", { transpose: 3 }), clip("c", { transpose: 47 })]);

  it("a relative transpose moves each clip from its base, clamped per clip, as one batch", () => {
    const clips = list();
    const c = offsetTranspose(clips, transposeBases(clips), 2);
    expect(c?.domain === "Edit" && c.command.type).toBe("Batch");
    expect(sets(c)).toEqual([
      ["a", -4],
      ["b", 5],
      ["c", 48],
    ]);
  });

  it("measures the offset from the gesture's start, not the current values", () => {
    const start = list();
    const bases = transposeBases(start);
    // Mid-drag the clips already moved by +1; the drag is now at +3.
    const now = audioClips([clip("a", { transpose: -5 }), clip("b", { transpose: 4 }), clip("c", { transpose: 48 })]);
    expect(sets(offsetTranspose(now, bases, 3))).toEqual([
      ["a", -3],
      ["b", 6],
    ]);
    // Back to the start: every moved clip returns to its own value.
    expect(sets(offsetTranspose(now, bases, 0))).toEqual([
      ["a", -6],
      ["b", 3],
      ["c", 47],
    ]);
  });

  it("sends nothing when no clip changes; one clip is a plain command", () => {
    const clips = audioClips([clip("a", { transpose: 48 }), clip("b", { transpose: 48 })]);
    expect(offsetTranspose(clips, transposeBases(clips), 5)).toBeNull();
    const one = audioClips([clip("a", { transpose: 48 }), clip("b", { transpose: 0 })]);
    expect(offsetTranspose(one, transposeBases(one), 5)).toEqual({ domain: "Clip", command: { type: "SetTranspose", id: "b", semitones: 5 } });
  });

  it("a typed value sets every clip (the ones already at it left out)", () => {
    expect(sets(setAllTranspose(list(), 3))).toEqual([
      ["a", 3],
      ["c", 3],
    ]);
  });

  it("relative gain clamps to -60..+24 dB per clip", () => {
    const clips = audioClips([clip("a", { gain: -59 }), clip("b", { gain: 20 })]);
    expect(sets(offsetGain(clips, gainBases(clips), 6))).toEqual([
      ["a", -53],
      ["b", 24],
    ]);
    expect(sets(offsetGain(clips, gainBases(clips), -3))).toEqual([
      ["a", -60],
      ["b", 17],
    ]);
  });

  it("common: the shared value, or null when mixed", () => {
    expect(common([2, 2])).toBe(2);
    expect(common([2, 3])).toBeNull();
    expect(common([])).toBeNull();
  });
});

describe("complex+ formant builders", () => {
  /** The (clip id, formants) pairs a builder's command sets. */
  const formants = (c: Command | null): Array<[string, WarpFormants | undefined]> => {
    if (!c) return [];
    if (c.domain === "Edit" && c.command.type === "Batch") return c.command.commands.flatMap((x) => formants(x));
    if (c.domain === "Warp" && c.command.type === "SetWarp") return [[c.command.clip, c.command.warp.formants]];
    return [];
  };
  const plus = { enabled: true, mode: "ComplexPlus" as const, source_bpm: 120 };
  const list = (): AudioClip[] =>
    audioClips([
      clip("a", { warp: plus }),
      clip("b", { warp: { ...plus, formants: { amount: 0.5, shift: 3, envelope: 64 } } }),
      clip("c", { warp: { ...plus, formants: { amount: 1, shift: 23, envelope: 250 } } }),
    ]);

  it("a clip without formant settings reads the defaults", () => {
    expect(formantsOf(list()[0]!)).toEqual(FORMANTS_DEFAULT);
  });

  it("a relative edit moves each clip from its own value, clamped per clip, other settings kept", () => {
    const l = list();
    const c = offsetFormant(l, "shift", formantBases(l, "shift"), 2);
    expect(formants(c)).toEqual([
      ["a", { amount: 1, shift: 2, envelope: 128 }],
      ["b", { amount: 0.5, shift: 5, envelope: 64 }],
      ["c", { amount: 1, shift: 24, envelope: 250 }],
    ]);
    expect(c?.domain === "Edit" && c.command.type === "Batch" && c.command.key?.id).toBe("undo.warp.set");
    // The warp itself is kept.
    const one = offsetFormant(l.slice(0, 1), "envelope", formantBases(l, "envelope"), -200);
    expect(one?.domain === "Warp" && one.command.type === "SetWarp" && one.command.warp).toEqual({ ...plus, formants: { amount: 1, shift: 0, envelope: 8 } });
  });

  it("a typed value sets every clip (clamped), leaving out the ones already there", () => {
    expect(formants(setAllFormant(list(), "amount", 0.5))).toEqual([
      ["a", { amount: 0.5, shift: 0, envelope: 128 }],
      ["c", { amount: 0.5, shift: 23, envelope: 250 }],
    ]);
    // Clamped to 100 %: only "b" (at 50 %) changes.
    expect(formants(setAllFormant(list(), "amount", 7)).map(([id, f]) => [id, f?.amount])).toEqual([["b", 1]]);
  });
});
