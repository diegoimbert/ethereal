/** The clip-editing layers' build output (instances per clip state) and hit tests, without GL. */
import { describe, expect, it } from "vitest";
import type { AudioContent, Clip, ClipId, ExpressionLane, Project } from "@/generated";
import { useProjectStore } from "@/state";
import { MockTransport } from "@/transport/mock";
import { CLIP_INSET, CLIP_RADIUS, CLIP_TITLE } from "../../laneBodyGeometry";
import { stableBoundsOf, type LaneItem } from "../../laneItems";
import type { LaneLayerContext, LayerOutput } from "../layers";
import type { Palette } from "../palette";
import { InstanceWriter } from "../pipelines";
import type { ArrangerRenderer } from "../renderer";
import type { Hover } from "../scene";
import { badgesOf } from "./clipBadges";
import { clipFadesLayer } from "./clipFades";
import { FADE_STRIDE, FadeMode, fadePipeline } from "./fadePipeline";
import { fadeGeometry, hitFadeHandle, laneCrossfades, visibleSpans, drawOrder } from "./fadeModel";

const PALETTE = {
  theme: "dark",
  bg: [0, 0, 0, 1],
  panel: [0.1, 0.1, 0.1, 1],
  titleText: [0, 0, 0, 1],
  seam: [1, 1, 1, 1],
  over: [1, 0, 0, 1],
  selection: [0.3, 0.5, 1, 1],
  white: [1, 1, 1, 1],
  black: [0, 0, 0, 1],
} as unknown as Palette;

function audio(id: string, start: number, length: number, a: Partial<AudioContent> = {}): Clip {
  return {
    id,
    track: "t",
    start,
    name: id,
    color: null,
    muted: false,
    length,
    offset: 0,
    looping: { enabled: false, start: 0, end: length },
    content: {
      type: "Audio",
      media: "m",
      gain: 0,
      transpose: 0,
      fade_in: 0,
      fade_out: 0,
      warp: { mode: "Off" },
      fade_in_curve: { type: "Linear" },
      fade_out_curve: { type: "Linear" },
      reversed: false,
      ...a,
    },
  } as unknown as Clip;
}

const item = (clip: Clip): LaneItem => ({ clip, bounds: stableBoundsOf(clip), dragging: false, ghost: false });

/** Builds the fades layer for one lane; returns the instances' modes per clip order. */
function build(clips: Clip[], opts: { selected?: ClipId[]; hover?: Hover | null; ppb?: number } = {}) {
  const pipeline = fadePipeline(CLIP_RADIUS);
  const layer = clipFadesLayer({} as ArrangerRenderer, new MockTransport(), pipeline);
  const items = clips.map(item);
  const ctx = {
    lane: { items, track: { id: "t", freeze: null }, staticVersion: 1 },
    input: { palette: PALETTE, selected: new Set(opts.selected ?? []), hover: opts.hover ?? null },
    window: { ppb: opts.ppb ?? 24, from: -100, to: 100, dpr: 1 },
  } as unknown as LaneLayerContext;
  const writer = new InstanceWriter(FADE_STRIDE);
  const out = { custom: () => writer } as unknown as LayerOutput;
  layer.build!(ctx, out);
  const data = writer.take();
  const inst = [];
  for (let o = 0; o < data.length; o += FADE_STRIDE) {
    inst.push({ box: [data[o], data[o + 1]], mode: data[o + 3], f0: data[o + 4], f1: data[o + 5], vis: [data[o + 8], data[o + 9]], dir: data[o + 10] });
  }
  return { inst, layer };
}

describe("clip fades layer", () => {
  it("draws the shade and curve of each fade, no handles at rest", () => {
    const { inst, layer } = build([audio("a", 0, 8, { fade_in: 1, fade_out: 2 })]);
    expect(inst.map((i) => i.mode)).toEqual([FadeMode.Shade, FadeMode.Line, FadeMode.Shade, FadeMode.Line]);
    expect(inst[0]).toMatchObject({ f0: 0, f1: 1, dir: 1 });
    expect(inst[2]).toMatchObject({ f0: 6, f1: 8, dir: -1 });
    expect(layer.decor("a")).toEqual({ lines: 2, crossfadeIn: false, crossfadeOut: false, handles: false });
  });

  it("shows the handles only while the clip is hovered (selected: its fades, no handles)", () => {
    const sel = build([audio("a", 0, 8, { fade_in: 1 })], { selected: ["a"] }).inst.map((i) => i.mode);
    expect(sel).toEqual([FadeMode.Shade, FadeMode.Line]);
    const hov = build([audio("a", 0, 8)], { hover: { clip: "a", part: "body" } }).inst.map((i) => i.mode);
    expect(hov).toEqual([FadeMode.Handle, FadeMode.Handle, FadeMode.CurveHandle, FadeMode.CurveHandle]);
  });

  it("draws a crossfade as a tint and an X, under the clip drawn above", () => {
    // A ends 0.5 into B; both fades cover the 1-beat overlap.
    const a = audio("a", 0, 4.5, { fade_out: 1 });
    const b = audio("b", 3.5, 4.5, { fade_in: 1, fade_in_curve: { type: "EqualPower" } });
    const { inst, layer } = build([a, b]);
    const ofB = inst.filter((i) => i.box[0] === 3.5);
    expect(ofB.map((i) => i.mode)).toEqual([FadeMode.Tint, FadeMode.Dashed, FadeMode.Line]);
    // The neighbour's (A's) fade-out across B's fade-in.
    expect(ofB[1]).toMatchObject({ f0: 3.5, f1: 4.5, dir: -1 });
    // A's overlay stops where B (above it) starts.
    const ofA = inst.filter((i) => i.box[0] === 0);
    expect(ofA.every((i) => i.vis[1] === 3.5)).toBe(true);
    expect(layer.decor("b")).toMatchObject({ crossfadeIn: true, lines: 2 });
    expect(layer.decor("a")).toMatchObject({ crossfadeOut: true, lines: 2 });
  });

  it("draws the hovered clip's handles whole and last, over the clip above it (a crossfade's left edge)", () => {
    const a = audio("a", 0, 4.5, { fade_out: 1 });
    const b = audio("b", 3.5, 4.5, { fade_in: 1 });
    const { inst } = build([a, b], { hover: { clip: "a", part: "header" } });
    const handles = inst.slice(-4);
    expect(handles.map((i) => i.mode)).toEqual([FadeMode.Handle, FadeMode.Handle, FadeMode.CurveHandle, FadeMode.CurveHandle]);
    expect(handles.every((i) => i.box[0] === 0 && i.vis[0]! < -1e6 && i.vis[1]! > 1e6)).toBe(true);
    // A's fade-out handle sits on B's start: not cut there.
    expect(handles[1]).toMatchObject({ f0: 3.5, f1: 3.5 });
  });

  it("skips small clips (painted without an overlay) and MIDI clips", () => {
    const midi = { ...audio("m", 0, 8), content: { type: "Midi" } } as unknown as Clip;
    expect(build([audio("a", 0, 0.1, { fade_in: 0.05 }), midi], { selected: ["a"] }).inst).toEqual([]);
  });
});

describe("fade model", () => {
  it("finds crossfades between neighbours only when the fades cover the overlap", () => {
    const a = audio("a", 0, 4.5, { fade_out: 1 });
    const b = audio("b", 3.5, 4.5, { fade_in: 1 });
    expect(laneCrossfades([item(a), item(b)]).get("b")?.in).toEqual({ type: "Linear" });
    const short = audio("b", 3.5, 4.5, { fade_in: 0.5 });
    expect(laneCrossfades([item(a), item(short)]).size).toBe(0);
  });

  it("clips the spans covered by later and raised clips", () => {
    const order = drawOrder([item(audio("a", 0, 8)), item(audio("b", 2, 2)), item(audio("c", 6, 4))], new Set(["b"]));
    expect(order.map((i) => i.clip.id)).toEqual(["a", "c", "b"]);
    expect(visibleSpans(order)[0]).toEqual([
      [-Infinity, 2],
      [4, 6],
    ]);
  });

  it("in a compact lane the fades span the whole clip (no title band above them)", () => {
    const it = item(audio("a", 0, 8, { fade_in: 2 }));
    const g = fadeGeometry(it, { scrollBeats: 0, pxPerBeat: 24 }, 36)!;
    expect(g.top).toBe(CLIP_INSET);
    expect(g.bodyH).toBe(36 - 2 * CLIP_INSET);
  });

  it("hit-tests handles with their priorities", () => {
    const it = item(audio("a", 0, 8, { fade_in: 2 }));
    const g = fadeGeometry(it, { scrollBeats: 0, pxPerBeat: 24 }, 60)!;
    const top = CLIP_INSET + CLIP_TITLE;
    // The length handle at the fade's end, its curve handle at the fade's middle.
    expect(hitFadeHandle(g, 48, top + 2)).toBe("fade-in");
    expect(hitFadeHandle(g, 24, g.inCurve!.y)).toBe("fade-in-curve");
    // Never over the title band.
    expect(hitFadeHandle(g, 48, top - 1)).toBeNull();
    // The resize strip keeps the lower part of the edge; the fade-out handle sits inset.
    expect(hitFadeHandle(g, 2, 50)).toBeNull();
    expect(hitFadeHandle(g, 8 * 24 - 4, top + 3)).toBe("fade-out");
  });
});

describe("clip badges", () => {
  it("lists REV and Missing", () => {
    expect(badgesOf(audio("a", 0, 1, { reversed: true }), new Set())).toEqual(["reversed"]);
    expect(badgesOf(audio("a", 0, 1, { reversed: true }), new Set(["m"]))).toEqual(["reversed", "missing"]);
    expect(badgesOf(audio("a", 0, 1), new Set())).toEqual([]);
  });

  it("a MIDI clip with controller data shows a controllers badge (empty lanes don't)", () => {
    const midi: Clip = { ...audio("m", 0, 4), content: { type: "Midi" } };
    const lane = (id: string, clip: string, n: number): ExpressionLane => ({
      id,
      clip,
      kind: { type: "Cc", controller: 64 },
      points: Array.from({ length: n }, (_, i) => ({ time: i, value: 1, curve: { type: "Step" as const } })),
    });
    const before = useProjectStore.getState().project;
    try {
      useProjectStore.setState({ project: { expression_lanes: { a: lane("a", "m", 0) } } as unknown as Project });
      expect(badgesOf(midi, new Set())).toEqual([]);
      useProjectStore.setState({ project: { expression_lanes: { a: lane("a", "m", 2), b: lane("b", "other", 2) } } as unknown as Project });
      expect(badgesOf(midi, new Set())).toEqual(["controllers"]);
    } finally {
      useProjectStore.setState({ project: before });
    }
  });
});
