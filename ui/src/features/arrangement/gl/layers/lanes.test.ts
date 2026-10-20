/** Build output of the WebGL arranger's lane layers (automation, comping, group summaries). */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AutomationPoint, CompRegion, Project, TakeLane, Track } from "@/generated";
import { FULL_RANGE } from "@/features/automation/valueAxis";
import { LANE_PAD } from "@/features/automation/geometry";
import { useCompingUi } from "@/features/comping/store";
import { TAKE_LANE_HEIGHT } from "@/features/comping/height";
import { createDemoProject, makeClip } from "@/transport/mock/demoProject";
import type { Row } from "../../layout";
import type { LaneLayerContext, LayerOutput } from "../layers";
import type { Palette } from "../palette";
import { InstanceWriter, RECT_STRIDE } from "../pipelines";
import type { LaneScene, SceneInput, ViewWindow } from "../scene";
import { automationLayer, automationPointAt, baselineDashes, buildAutomationLane, envelope, type GlAutomationLane } from "./automationLanes";
import { compingLayers, compItems, MUTED_OPACITY } from "./comping";
import { groupSummaryLayer } from "./groupSummary";
import { lanePalette } from "./lanePalette";
import { SHAPE_STRIDE, shapePipeline } from "./shapes";
import { WindowTokens } from "./viewWindow";

const palette: Palette = {
  theme: "dark",
  bg: [0.1, 0.1, 0.1, 1],
  panel: [0.15, 0.15, 0.15, 1],
  titleText: [0, 0, 0, 1],
  seam: [0, 0, 0, 0.3],
  over: [1, 0, 0, 1],
  selection: [1, 0, 1, 1],
  white: [1, 1, 1, 1],
  black: [0, 0, 0, 1],
};

const shape = shapePipeline();

function output() {
  const rects = new InstanceWriter(RECT_STRIDE);
  const shapes = new InstanceWriter(SHAPE_STRIDE);
  const out = { rects, text: new InstanceWriter(24), notes: new InstanceWriter(16), atlas: null as never, custom: () => shapes } as unknown as LayerOutput;
  return { out, rects, shapes };
}

/** The `i`th shape instance: [beatA, pxA, beatB, pxB, yA, yB, radius, stroke width, …fill, …stroke, mode, …]. */
const shapeAt = (w: InstanceWriter, i: number) => Array.from(w.data.subarray(i * SHAPE_STRIDE, (i + 1) * SHAPE_STRIDE));
const rectAt = (w: InstanceWriter, i: number) => Array.from(w.data.subarray(i * RECT_STRIDE, (i + 1) * RECT_STRIDE));

const pt = (id: string, time: number, value: number, curve: AutomationPoint["curve"] = { type: "Linear" }): AutomationPoint =>
  ({ id, lane: "l", time, value, curve }) as AutomationPoint;

const win = (from: number, to: number, ppb = 10): ViewWindow => ({ from, to, ppb, dpr: 1 });

describe("automation envelope", () => {
  const y = (v: number) => 100 - v * 100;
  it("holds before the first and after the last point, straight linear segments", () => {
    const s = envelope([pt("a", 1, 0.5), pt("b", 3, 1)], 0, y, 0, 10, 10);
    expect(s).toEqual([-1e6, 50, 1, 50, 1, 50, 3, 0, 3, 0, 1e6, 0]);
  });

  it("steps hold then jump", () => {
    const s = envelope([pt("a", 1, 0, { type: "Step" }), pt("b", 2, 1)], 0, y, 0, 10, 10);
    expect(s.slice(4, 12)).toEqual([1, 100, 2, 100, 2, 100, 2, 0]);
  });

  it("samples curves every 2 px with the engine formula", () => {
    const s = envelope([pt("a", 0, 0, { type: "Curve", tension: 1 }), pt("b", 4, 1)], 0, y, 0, 10, 10);
    // 4 beats at 10 px/beat: 20 pieces, after the leading hold.
    const curve = s.slice(4, s.length - 4);
    expect(curve.length / 4).toBe(20);
    // Halfway: x^(4^1) = 0.0625.
    const mid = curve.findIndex((v, i) => i % 4 === 2 && Math.abs(v - 2) < 1e-9);
    expect(curve[mid + 1]).toBeCloseTo(y(0.0625), 9);
  });

  it("only builds the segments reaching the window (10k points)", () => {
    const pts = Array.from({ length: 10_000 }, (_, i) => pt(`p${i}`, i, (i % 7) / 7));
    const s = envelope(pts, 0, y, 5000, 5100, 10);
    // Segments 4999..5100 plus nothing past.
    expect(s.length / 4).toBe(101); // 4999→5000 … 5099→5100
    expect(s[0]).toBe(4999);
  });
});

describe("automation lane build", () => {
  const lane = (over: Partial<GlAutomationLane> = {}): GlAutomationLane => ({
    track: "t",
    el: document.createElement("div"),
    points: [pt("a", 1, 0.25), pt("b", 2, 0.75), pt("c", 3, 0.5)],
    selected: new Set(["b"]),
    height: 64,
    range: FULL_RANGE,
    lines: [
      { value: 0.5, label: null, major: true },
      { value: 0.25, label: null, major: false },
    ],
    disabled: false,
    offset: 0,
    ...over,
  });
  const place = { top: 100, clipTop: 100, clipBottom: 164 };
  const ctx = { input: { palette } as SceneInput };

  it("writes the background, step lines, envelope and points", () => {
    const { out, rects, shapes } = output();
    buildAutomationLane(lane(), place, { y0: 56, y1: 300 }, win(0, 10), ctx, out, shape);
    // Background + 2 step lines.
    expect(rects.count).toBe(3);
    const c = lanePalette(palette);
    expect(
      rectAt(rects, 0)
        .slice(12, 16)
        .map((v) => +v.toFixed(5)),
    ).toEqual([...c.bgInset].map((v) => +v.toFixed(5)));
    // A step line 1 px tall, centered on its value (y in the row).
    const step = rectAt(rects, 1);
    const yy = 100 + LANE_PAD + (64 - 2 * LANE_PAD) * 0.5;
    expect([step[8], step[10]]).toEqual([yy - 0.5, yy + 0.5]);
    // 4 envelope pieces (hold, 2 lines, hold) + 3 points.
    expect(shapes.count).toBe(7);
    const selected = shapeAt(shapes, 5);
    expect(selected[6]).toBe(4.75); // r 4 + half the 1.5 stroke
    expect(selected[7]).toBe(1.5);
    expect(selected.slice(8, 12).map((v) => +v.toFixed(5))).toEqual([...c.selection].map((v) => +v.toFixed(5)));
    // Everything clipped to the lane.
    expect(shapeAt(shapes, 0).slice(20, 22)).toEqual([100, 164]);
  });

  it("dims a disabled lane against its background", () => {
    const { out, shapes } = output();
    buildAutomationLane(lane({ disabled: true }), place, { y0: 0, y1: 300 }, win(0, 10), ctx, out, shape);
    const c = lanePalette(palette);
    const line = shapeAt(shapes, 0).slice(8, 12);
    for (let i = 0; i < 3; i++) expect(line[i]).toBeCloseTo(c.accent[i]! * 0.35 + c.bgInset[i]! * 0.65, 5);
    expect(line[3]).toBeCloseTo(1, 5);
  });

  it("clips to the animating slot", () => {
    const { out, rects } = output();
    buildAutomationLane(lane(), place, { y0: 56, y1: 130 }, win(0, 10), ctx, out, shape);
    const bg = rectAt(rects, 0);
    expect([bg[8], bg[10]]).toEqual([100, 130]);
    const none = output();
    buildAutomationLane(lane(), place, { y0: 56, y1: 90 }, win(0, 10), ctx, none.out, shape);
    expect(none.rects.count).toBe(0);
  });

  it("an empty lane draws its baseline: gray dashes at the current value's y", () => {
    const { out, shapes } = output();
    buildAutomationLane(lane({ points: [], baseline: 0.75 }), place, { y0: 0, y1: 300 }, win(0, 10, 10), ctx, out, shape);
    // 10 beats at 10 px/beat = 100 px: 4 px dashes every 8 px, anchored at beat 0.
    expect(shapes.count).toBe(13);
    const c = lanePalette(palette);
    const yy = 100 + LANE_PAD + (64 - 2 * LANE_PAD) * 0.25;
    const first = shapeAt(shapes, 0);
    // Dash 0: beats [0, 0.4], inset by the 0.75 px cap radius.
    expect(first.slice(0, 6).map((v) => +v.toFixed(5))).toEqual([0, 0.75, 0.4, -0.75, yy, yy]);
    expect(first.slice(8, 12).map((v) => +v.toFixed(5))).toEqual([...c.textDim].map((v) => +v.toFixed(5)));
    expect(shapeAt(shapes, 1)[0]).toBeCloseTo(0.8, 6);
  });

  it("a lane with points draws no baseline", () => {
    const { out, shapes } = output();
    buildAutomationLane(lane({ baseline: 0.75 }), place, { y0: 0, y1: 300 }, win(0, 10), ctx, out, shape);
    expect(shapes.count).toBe(7);
  });

  it("baseline dashes are anchored at beat 0 and cover the window", () => {
    expect(baselineDashes(0, 1, 8)).toEqual([0, 0.5]);
    const d = baselineDashes(1.2, 3, 4); // 1 beat dashes, period 2
    expect(d).toEqual([0, 1, 2, 3]);
    expect(baselineDashes(0, 0, 4)).toEqual([]);
  });

  it("hit-tests points (topmost wins)", () => {
    const l = lane();
    const v = { scrollBeats: 0, pxPerBeat: 40 };
    const y = LANE_PAD + (64 - 2 * LANE_PAD) * 0.25; // value 0.75
    expect(automationPointAt(l, 80, y, v)?.id).toBe("b");
    expect(automationPointAt(l, 84, y + 2, v)?.id).toBe("b");
    expect(automationPointAt(l, 86, y, v)).toBeNull();
  });

  it("the layer rebuilds when a lane is published or the view leaves its window", () => {
    const layer = automationLayer(shape, () => place);
    const row = { track: { id: "t" }, y: 0, laneHeight: 56, height: 140, depth: 0 } as unknown as Row;
    const base = { row, track: row.track, input: { palette } as SceneInput, lane: {} as LaneScene };
    const a = layer.deps!({ ...base, window: win(0, 30) } as LaneLayerContext);
    const b = layer.deps!({ ...base, window: win(1, 31) } as LaneLayerContext);
    expect(b).toEqual(a);
    const c = layer.deps!({ ...base, window: win(15, 45) } as LaneLayerContext);
    expect(c[4]).not.toBe(a[4]);
  });
});

describe("view windows", () => {
  it("keep the built window while the view stays inside it at the same zoom", () => {
    const w = new WindowTokens();
    const first = win(-10, 20);
    expect(w.token("k", first)).toBe(first);
    expect(w.token("k", win(-5, 25))).toBe(first);
    expect(w.token("k", win(-10, 20, 20))).not.toBe(first);
  });
});

describe("group summary", () => {
  it("draws the merged spans of the group's tracks at 40–60 % of the lane", () => {
    const p = createDemoProject();
    const group = { id: "g", kind: "Group", color: 0xff0000, parent: null } as unknown as Track;
    const kid = { id: "k", kind: "Midi", color: 0, parent: "g" } as unknown as Track;
    const project = {
      ...p,
      tracks: { g: group, k: kid },
      clips: {
        c1: makeClip({ id: "c1", track: "k", start: 0, length: 4 }),
        c2: makeClip({ id: "c2", track: "k", start: 2, length: 4 }),
        c3: makeClip({ id: "c3", track: "k", start: 10, length: 1 }),
      },
    } as Project;
    const { out, rects } = output();
    const row = { track: group, y: 0, laneHeight: 56, height: 56, depth: 0 } as unknown as Row;
    groupSummaryLayer().build!({ row, track: group, input: { project, palette } as SceneInput, lane: {} as LaneScene, window: win(0, 10) }, out);
    expect(rects.count).toBe(2);
    const bar = rectAt(rects, 0);
    expect(bar.slice(0, 2)).toEqual([0, 6]);
    expect(bar.slice(8, 12).map((v) => +v.toFixed(5))).toEqual([0, 0.4, 0, 0.6]);
    expect(bar.slice(12, 16).map((v) => +v.toFixed(3))).toEqual([1, 0, 0, 0.5]);
  });
});

describe("comping", () => {
  const track = { id: "t", kind: "Audio", color: 0x00ff00, parent: null } as unknown as Track;
  const lanes: Record<string, TakeLane> = {
    la: { id: "la", track: "t", order: "a", name: "Take 1", color: null },
    lb: { id: "lb", track: "t", order: "b", name: "Take 2", color: 0x0000ff },
  };
  const regions: Record<string, CompRegion> = {
    r1: { id: "r1", track: "t", lane: "la", start: 0, end: 2, crossfade: 0 },
    r2: { id: "r2", track: "t", lane: "lb", start: 2, end: 4, crossfade: 0 },
  };
  const project = {
    ...createDemoProject(),
    tracks: { t: track },
    take_lanes: lanes,
    comp_regions: regions,
    clips: {
      ca: makeClip({ id: "ca", track: "t", start: 0, length: 4, lane: "la", name: "A" }),
      cb: makeClip({ id: "cb", track: "t", start: 0, length: 4, lane: "lb", name: "B" }),
    },
  } as Project;
  const row = {
    track,
    y: 0,
    laneHeight: 56 + 2 * TAKE_LANE_HEIGHT,
    takesHeight: 2 * TAKE_LANE_HEIGHT,
    height: 56 + 2 * TAKE_LANE_HEIGHT,
    depth: 0,
  } as unknown as Row;
  const ctx = (): LaneLayerContext => ({
    row,
    track,
    input: { project, palette, tempo: null } as unknown as SceneInput,
    lane: {} as LaneScene,
    window: win(0, 10),
  });

  beforeEach(() => useCompingUi.setState({ expanded: new Set(["t"]), selected: { track: "t", region: "r2" }, swipe: null, audition: new Map() }));
  afterEach(() => useCompingUi.setState({ expanded: new Set(), selected: null, swipe: null, audition: new Map() }));

  it("shapes the comp's pieces from their take clips, named after the take", () => {
    const items = compItems(project, track, null);
    expect(items.map((i) => [i.region, i.clip.id, i.bounds.start, i.bounds.length, i.clip.name])).toEqual([
      ["r1", "ca", 0, 2, "Take 1"],
      ["r2", "cb", 2, 2, "Take 2"],
    ]);
  });

  it("adds the comp over the main lane and the take lanes under it, dimmed unless auditioned", () => {
    const { over } = compingLayers(shape);
    let extra = over.lanes!(ctx());
    expect(extra.map((l) => [l.key, l.top, l.height, l.alpha])).toEqual([
      ["comp:t", 0, 56, 1],
      ["take:la", 56, TAKE_LANE_HEIGHT, MUTED_OPACITY],
      ["take:lb", 56 + TAKE_LANE_HEIGHT, TAKE_LANE_HEIGHT, MUTED_OPACITY],
    ]);
    // The same sources frame after frame (the renderer keeps their scenes by them).
    expect(over.lanes!(ctx())[1]!.source).toBe(extra[1]!.source);
    useCompingUi.getState().setAudition("t", "lb");
    extra = over.lanes!(ctx());
    expect(extra.map((l) => l.alpha)).toEqual([MUTED_OPACITY, MUTED_OPACITY, 1]);
    expect(extra[2]!.source.items(ctx().input).map((i) => i.clip.id)).toEqual(["cb"]);
  });

  it("outlines the pieces (dashed; the selected region's solid), marks regions and the swipe", () => {
    const { over } = compingLayers(shape);
    useCompingUi.getState().setSwipe({ track: "t", lane: "la", start: 1, end: 3 });
    const { out, rects, shapes } = output();
    over.build!(ctx(), out);
    expect(shapes.count).toBe(2);
    const dashed = shapeAt(shapes, 0);
    const solid = shapeAt(shapes, 1);
    expect([dashed[16], dashed[17], dashed[7]]).toEqual([1, 3, 1]); // outline, 3 px dashes, 1 px
    expect([solid[16], solid[17], solid[7]]).toEqual([1, 0, 2]); // outline, solid, 2 px
    // Region marks per take lane, then its swipe (r2 selected: its ring first).
    expect(rects.count).toBe(4);
    const swipe = rectAt(rects, 1);
    expect(swipe.slice(0, 2)).toEqual([1, 3]);
    expect([swipe[8], swipe[10]]).toEqual([56, 56 + TAKE_LANE_HEIGHT]);
  });

  it("a click on a piece selects its region and opens the take lanes", () => {
    const { over } = compingLayers(shape);
    useCompingUi.setState({ expanded: new Set(), selected: null });
    // No renderer mounted: no hit (the test API and gestures go through the renderer).
    expect(over.hitTest!(10, 20, row)).toBeNull();
    const e = { stopPropagation: () => {} } as PointerEvent;
    over.onPointerDown!({ clip: "ca", track: "t", part: "layer", layer: "comping", data: { region: "r1" } }, e);
    expect(useCompingUi.getState().selected).toEqual({ track: "t", region: "r1" });
    expect(useCompingUi.getState().expanded.has("t")).toBe(true);
  });
});
