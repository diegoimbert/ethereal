// Pure parts of the WebGL arranger: hit-test priorities, culling, color mixes, instance writers.
import { describe, expect, it } from "vitest";
import type { Clip } from "@/generated";
import type { LaneItem } from "../laneItems";
import { boxX, cursorFor, EDGE_HANDLE_MIN_PX, EDGE_HANDLE_PX, handleWidth, hitLane, MIN_RESIZE_PX } from "./hitTest";
import { clipColors, compactNameColor, mix, parseColor, rgbOf, type Palette } from "./palette";
import { InstanceWriter, RECT_STRIDE, RectFlag, writeRect } from "./pipelines";
import { cullRange, type CullIndex } from "./scene";

const clip = (id: string, start: number, length: number): Clip =>
  ({
    id,
    track: "t",
    start,
    length,
    offset: 0,
    name: id,
    color: null,
    muted: false,
    lane: null,
    looping: { enabled: false, start: 0, end: length },
    content: { type: "Midi" },
  }) as unknown as Clip;

const item = (c: Clip, extra: Partial<LaneItem> = {}): LaneItem => ({
  clip: c,
  bounds: { track: c.track, start: c.start, length: c.length, offset: c.offset },
  dragging: false,
  ghost: false,
  ...extra,
});

const view = { scrollBeats: 0, pxPerBeat: 20 };
const H = 56;

describe("hitLane", () => {
  const a = item(clip("a", 0, 4)); // x 0..80
  const b = item(clip("b", 4, 4)); // x 80..160
  const none = new Set<string>();

  it("edge handles, then the title band, then the body", () => {
    expect(hitLane([a, b], none, "t", 2, 30, H, view)?.part).toBe("resize-start");
    expect(hitLane([a, b], none, "t", 78, 30, H, view)).toEqual({ clip: "a", track: "t", part: "resize-end" });
    expect(hitLane([a, b], none, "t", 40, 8, H, view)).toEqual({ clip: "a", track: "t", part: "header" });
    expect(hitLane([a, b], none, "t", 40, 30, H, view)?.part).toBe("body");
    // Outside the clip box (its 2 px inset) or the lane: nothing.
    expect(hitLane([a, b], none, "t", 40, 1, H, view)).toBeNull();
    expect(hitLane([a, b], none, "t", 40, 60, H, view)).toBeNull();
  });

  it("in a compact lane the whole clip is its title band (no body)", () => {
    expect(hitLane([a, b], none, "t", 40, 30, 36, view)).toEqual({ clip: "a", track: "t", part: "header" });
  });

  it("handles are 6 px wide (a quarter of the clip at most)", () => {
    const n = item(clip("n", 0, 3)); // 60 px
    expect(hitLane([n], none, "t", 5, 8, H, view)?.part).toBe("resize-start");
    expect(hitLane([n], none, "t", 7, 8, H, view)?.part).toBe("header");
  });

  it("selected and dragged clips are above the others; ghosts are never hit", () => {
    const over = item(clip("o", 2, 4)); // overlaps a: x 40..120
    expect(hitLane([a, over], none, "t", 60, 8, H, view)?.clip).toBe("o");
    expect(hitLane([a, over], new Set(["a"]), "t", 60, 8, H, view)?.clip).toBe("a");
    const ghost = item(clip("g", 0, 4), { ghost: true });
    expect(hitLane([ghost], none, "t", 40, 8, H, view)).toBeNull();
  });

  it("a body is reported only when nothing interactive is hit", () => {
    const tall = item(clip("x", 0, 4));
    const top = item(clip("y", 1, 4));
    // In the body band of both overlapping clips: the topmost body.
    expect(hitLane([tall, top], none, "t", 30, 30, H, view)).toEqual({ clip: "y", track: "t", part: "body" });
  });

  it("small clips (< 48 px) are hit anywhere in the lane's height, with slop", () => {
    const s = item(clip("s", 10, 1)); // 200..220 px
    expect(hitLane([s], none, "t", 210, 50, H, view)).toEqual({ clip: "s", track: "t", part: "small" });
    expect(hitLane([s], none, "t", 222, 50, H, view)?.clip).toBe("s");
    expect(hitLane([s], none, "t", 230, 50, H, view)).toBeNull();
  });

  it("small clips keep their edge handles from 10 px", () => {
    const s = item(clip("s", 10, 1)); // 200..220 px
    expect(hitLane([s], none, "t", 201, 50, H, view)).toEqual({ clip: "s", track: "t", part: "resize-start" });
    expect(hitLane([s], none, "t", 219, 50, H, view)).toEqual({ clip: "s", track: "t", part: "resize-end" });
    // The slop past the edges moves.
    expect(hitLane([s], none, "t", 221, 50, H, view)?.part).toBe("small");
    expect(hitLane([s], none, "t", 198, 50, H, view)?.part).toBe("small");
  });

  it("boxes are at least 8 px wide", () => {
    expect(boxX(item(clip("z", 0, 0.01)), view)).toEqual([0, 8]);
  });

  it("cursors per part", () => {
    expect(cursorFor("header")).toBe("grab");
    expect(cursorFor("resize-end")).toBe("ew-resize");
    expect(cursorFor("body")).toBe("");
  });
});

describe("edge handles by clip width", () => {
  const none = new Set<string>();
  /** The part at each px across a clip at x 0, `w` px wide at `pxPerBeat`. */
  const parts = (w: number, pxPerBeat = w) => {
    const v = { scrollBeats: 0, pxPerBeat };
    const c = item(clip("c", 0, w / pxPerBeat));
    return Array.from({ length: Math.ceil(w) }, (_, x) => hitLane([c], none, "t", x + 0.5, 30, H, v)?.part);
  };
  const count = (ps: ReturnType<typeof parts>, part: string) => ps.filter((p) => p === part).length;

  it("widths: 25% of the box, clamped to 3..6 px, none under 10 px", () => {
    expect(MIN_RESIZE_PX).toBe(10);
    expect(handleWidth(0, 6)).toBe(0);
    expect(handleWidth(0, 9.9)).toBe(0);
    expect(handleWidth(0, 10)).toBe(EDGE_HANDLE_MIN_PX);
    expect(handleWidth(0, 12)).toBe(3);
    expect(handleWidth(0, 20)).toBe(5);
    expect(handleWidth(0, 40)).toBe(EDGE_HANDLE_PX);
    expect(handleWidth(0, 200)).toBe(6);
  });

  it.each([
    // [width px, handle px, grab zone px]
    [12, 3, 6],
    [20, 5, 10],
    [24, 6, 12],
    [40, 6, 28],
    [200, 6, 188],
  ])("a %d px clip has %d px handles and a %d px grab zone", (w, hw, mid) => {
    const ps = parts(w);
    expect(ps[0]).toBe("resize-start");
    expect(ps[w - 1]).toBe("resize-end");
    expect(count(ps, "resize-start")).toBe(hw);
    expect(count(ps, "resize-end")).toBe(hw);
    expect(ps.length - 2 * hw).toBe(mid);
    // The rest moves: a small clip's whole height, else the body below the title band.
    expect(ps.slice(hw, w - hw).every((p) => p === (w < 48 ? "small" : "body"))).toBe(true);
  });

  it("a 6 px clip only moves (no handles)", () => {
    expect(parts(6).every((p) => p === "small")).toBe(true);
  });

  it("depends on the clip's width on screen, at any zoom", () => {
    expect(count(parts(6, 6), "resize-end")).toBe(0);
    expect(count(parts(24, 24), "resize-end")).toBe(6);
    expect(count(parts(200, 200), "resize-end")).toBe(6);
    // 4 beats at 3 px/beat (12 px) and half a beat at 80 px/beat (40 px).
    expect(count(parts(12, 3), "resize-start")).toBe(3);
    expect(count(parts(40, 80), "resize-start")).toBe(6);
  });

  it("cursors follow the hit test", () => {
    const ps = parts(20);
    expect(cursorFor(ps[0]!)).toBe("ew-resize");
    expect(cursorFor(ps[10]!)).toBe("");
    expect(cursorFor(ps[19]!)).toBe("ew-resize");
  });
});

describe("cullRange", () => {
  const ix = (starts: number[], ends: number[]): CullIndex => {
    let m = -Infinity;
    return {
      starts: Float64Array.from(starts),
      maxEnds: Float64Array.from(ends.map((e) => (m = Math.max(m, e)))),
      under: new Int32Array(starts.length + 1),
      notes: new Int32Array(starts.length + 1),
      over: new Int32Array(starts.length + 1),
    };
  };

  it("the clips overlapping the window, a long earlier clip included", () => {
    const i = ix([0, 2, 10, 20, 30], [40, 4, 12, 22, 32]);
    // The long first clip reaches into any window: everything from it to the last start.
    expect(cullRange(i, 15, 25)).toEqual([0, 4]);
    const j = ix([0, 2, 10, 20, 30], [1, 4, 12, 22, 32]);
    expect(cullRange(j, 11, 25)).toEqual([2, 4]);
    expect(cullRange(j, 50, 60)).toEqual([5, 5]);
    expect(cullRange(j, -5, -1)).toEqual([0, 0]);
  });
});

describe("palette", () => {
  it("parses hex and rgb()", () => {
    expect(parseColor("#ff8000")).toEqual([1, 128 / 255, 0, 1]);
    expect(parseColor("rgba(0, 0, 0, 0.25)")).toEqual([0, 0, 0, 0.25]);
  });

  it("mixes like color-mix(in srgb)", () => {
    expect(mix([1, 0, 0, 1], 0.26, [0, 0, 1, 1]).map((v) => +v.toFixed(3))).toEqual([0.26, 0, 0.74, 1]);
  });

  it("clip colors: darker in the light theme, no glow there", () => {
    const base = {
      bg: [0, 0, 0, 1],
      panel: [0.1, 0.1, 0.1, 1],
      titleText: [0, 0, 0, 1],
      seam: [0, 0, 0, 0.25],
      over: [1, 0, 0, 1],
      selection: [1, 0, 1, 1],
      white: [1, 1, 1, 1],
      black: [0, 0, 0, 1],
    } as const;
    const dark = clipColors({ ...base, theme: "dark" } as unknown as Palette, 0xff0000);
    const light = clipColors({ ...base, theme: "light" } as unknown as Palette, 0xff0000);
    // The color as shown (an off-palette red is softened: displayColor).
    const red = rgbOf(0xff0000);
    expect(dark.strong).toEqual(red);
    expect(light.strong.map((v) => +v.toFixed(2))).toEqual([...red.slice(0, 3).map((v) => +(v * 0.7).toFixed(2)), 1]);
    expect(dark.glowInner).not.toBeNull();
    expect(light.glowInner).toBeNull();
  });

  it("compact clip name: brightened like the border when selected", () => {
    const base = {
      bg: [0, 0, 0, 1],
      panel: [0.1, 0.1, 0.1, 1],
      titleText: [0, 0, 0, 1],
      seam: [0, 0, 0, 0.25],
      over: [1, 0, 0, 1],
      selection: [1, 0, 1, 1],
      white: [1, 1, 1, 1],
      black: [0, 0, 0, 1],
    } as const;
    const luma = (c: readonly [number, number, number, number]) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for (const theme of ["dark", "light"] as const) {
      const c = clipColors({ ...base, theme } as unknown as Palette, 0x3366cc);
      expect(compactNameColor(c, false)).toEqual(c.strong);
      // The selected name takes the selection rim's color (the border's).
      expect(compactNameColor(c, true)).toEqual(c.rim);
    }
    // Dark theme: the selected name is brighter than the unselected one.
    const dark = clipColors({ ...base, theme: "dark" } as unknown as Palette, 0x3366cc);
    expect(luma(compactNameColor(dark, true))).toBeGreaterThan(luma(compactNameColor(dark, false)));
  });
});

describe("instance writers", () => {
  it("writes a rect instance (32 floats) and grows", () => {
    const w = new InstanceWriter(RECT_STRIDE, 1);
    for (let i = 0; i < 3; i++) {
      writeRect(w, { x: { b0: i, b1: i + 1, minW: 8 }, y: { a0: 2, a1: -2, f1: 1 }, radius: 6, fill: [1, 0, 0, 1], flags: RectFlag.Snap });
    }
    expect(w.count).toBe(3);
    const d = w.take();
    expect(d.length).toBe(3 * RECT_STRIDE);
    expect([...d.subarray(32, 40)]).toEqual([1, 2, 0, 0, 8, 6, 0, RectFlag.Snap | RectFlag.NoClip]);
  });
});
