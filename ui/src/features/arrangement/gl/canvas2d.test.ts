// The Canvas2D fallback backend: what it paints for the instance data of each pipeline,
// against a recording 2D context (jsdom has none).
import { describe, expect, it } from "vitest";
import type { Track } from "@/generated";
import type { Row } from "../layout";
import { CLIP_INSET, CLIP_TITLE } from "../laneBodyGeometry";
import type { FrameSetup } from "./backend";
import { Canvas2DBackend, clippedShape } from "./canvas2d";
import { Batch, Pipeline } from "./glContext";
import { GlyphAtlas } from "./glyphAtlas";
import type { Palette } from "./palette";
import {
  Fx,
  InstanceWriter,
  NOTE_STRIDE,
  notePipeline,
  RECT_STRIDE,
  rectPipeline,
  RectFlag,
  TEXT_STRIDE,
  textPipeline,
  writeGlyph,
  writeNote,
  writeRect,
} from "./pipelines";
import type { FontSpec, RasterRun, TextRasterizer } from "./textRaster";
import { WaveTiles } from "./waveTiles";

type Call = { op: string; args: unknown[]; state: Record<string, unknown> };

/** A 2D context recording its calls (with the style state at each). */
function recorder(): { ctx: CanvasRenderingContext2D; calls: Call[] } {
  const calls: Call[] = [];
  const state: Record<string, unknown> = { globalAlpha: 1 };
  const stack: Array<Record<string, unknown>> = [];
  const ctx = new Proxy(
    {},
    {
      get(_t, key: string) {
        if (key in state) return state[key];
        if (key === "save") return () => stack.push({ ...state });
        if (key === "restore") return () => Object.assign(state, stack.pop() ?? {});
        if (key === "getImageData") return (_x: number, _y: number, w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) });
        return (...args: unknown[]) => calls.push({ op: key, args, state: { ...state } });
      },
      set(_t, key: string, value) {
        state[key] = value;
        return true;
      },
    },
  ) as CanvasRenderingContext2D;
  return { ctx, calls };
}

class FakeRaster implements TextRasterizer {
  reset(): void {}
  measure(text: string): number {
    return text.length * 6;
  }
  metrics(): { ascent: number; descent: number } {
    return { ascent: 10, descent: 2 };
  }
  draw(text: string): RasterRun {
    const width = text.length * 6;
    return { data: new Uint8Array(width * 12).fill(200), width, height: 12, originX: 0, baseline: 10, advance: width };
  }
  icon(_paths: ReadonlyArray<string>, px: number): Uint8Array {
    return new Uint8Array(px * px);
  }
}

const palette = { bg: [0, 0, 0, 1], over: [1, 0, 0, 1] } as unknown as Palette;
const frame: FrameSetup = { width: 400, height: 200, cssWidth: 400, cssHeight: 200, dpr: 1, scrollBeats: 0, pxPerBeat: 20, originY: 0, originX: 0, palette };
const row: Row = { track: { id: "t" } as Track, depth: 0, y: 10, laneHeight: 56, height: 56 };

function backend(atlas = new GlyphAtlas(new FakeRaster(), { width: 256, height: 64 })) {
  const rec = recorder();
  const canvas = { getContext: () => rec.ctx, addEventListener() {}, removeEventListener() {} } as unknown as HTMLCanvasElement;
  const b = new Canvas2DBackend(canvas, atlas, new WaveTiles());
  b.begin(frame);
  return { b, calls: rec.calls, atlas };
}

function batch(p: Pipeline, w: InstanceWriter): Batch {
  const out = new Batch(p);
  out.set(w.take());
  return out;
}

describe("clippedShape", () => {
  const clip = { x0: 0, y0: 0, x1: 100, y1: 50 };
  it("a title band in a rounded clip box: the box's radius at the corners they share", () => {
    expect(clippedShape({ x0: 0, y0: 0, x1: 100, y1: 16 }, 0, clip, 6)).toEqual({ box: { x0: 0, y0: 0, x1: 100, y1: 16 }, radii: [6, 6, 0, 0] });
  });
  it("cut to the clip box", () => {
    expect(clippedShape({ x0: -20, y0: 10, x1: 30, y1: 20 }, 0, clip, 6)).toEqual({ box: { x0: 0, y0: 10, x1: 30, y1: 20 }, radii: [0, 0, 0, 0] });
    expect(clippedShape({ x0: 200, y0: 0, x1: 300, y1: 10 }, 0, clip, 6)).toBe("empty");
  });
  it("a shape the box cuts through a rounded corner needs a clip path", () => {
    expect(clippedShape({ x0: 2, y0: 0, x1: 50, y1: 10 }, 0, clip, 6)).toBeNull();
    expect(clippedShape({ x0: -5, y0: 10, x1: 50, y1: 20 }, 4, clip, 6)).toBeNull();
  });
  it("unclipped", () => {
    const r = { x0: 5, y0: 5, x1: 10, y1: 10 };
    expect(clippedShape(r, 3, null, 0)).toEqual({ box: r, radii: [3, 3, 3, 3] });
  });
});

describe("Canvas2DBackend", () => {
  it("clip boxes: rounded fills at their lane position, title bands clipped to the box's corners", () => {
    const { b, calls } = backend();
    const w = new InstanceWriter(RECT_STRIDE);
    const box = { b0: 1, b1: 5, minW: 8 };
    writeRect(w, { x: box, y: { a0: 2, a1: -2, f1: 1 }, radius: 6, fill: [0, 0.5, 1, 1], flags: RectFlag.Snap });
    writeRect(w, { x: box, y: { a0: 2, a1: 2 + 16 }, fill: [1, 1, 1, 1], flags: RectFlag.Snap, clip: { x: box, y: { a0: 2, a1: -2, f1: 1 }, radius: 6 } });
    const p = rectPipeline();
    expect(b.draw(p, batch(p, w), row, 0, 2)).toBe(true);
    const rects = calls.filter((c) => c.op === "roundRect");
    expect(rects[0]!.args).toEqual([20, 12, 80, 52, [6, 6, 6, 6]]);
    expect(rects[1]!.args).toEqual([20, 12, 80, 16, [6, 6, 0, 0]]);
    const fills = calls.filter((c) => c.op === "fill").map((c) => c.state.fillStyle);
    expect(fills).toEqual(["rgba(0,128,255,1.0000)", "rgba(255,255,255,1.0000)"]);
  });

  it("the glow is a shadow outside the box, clipped to the lane", () => {
    const { b, calls } = backend();
    const w = new InstanceWriter(RECT_STRIDE);
    writeRect(w, { x: { b0: 1, b1: 5 }, y: { a0: 2, a1: -2, f1: 1 }, radius: 6, blur: 14, fill: [0, 1, 0, 0.5], flags: RectFlag.ClipLane });
    const p = rectPipeline();
    b.draw(p, batch(p, w), row, 0, 1);
    const fill = calls.find((c) => c.op === "fill")!;
    expect(fill.state.shadowBlur).toBe(14);
    expect(fill.state.shadowColor).toBe("rgba(0,255,0,0.5000)");
    expect(calls.find((c) => c.op === "clip")!.args).toEqual(["evenodd"]);
  });

  it("muted clips are desaturated and mixed into the background", () => {
    const { b, calls } = backend();
    const w = new InstanceWriter(RECT_STRIDE);
    writeRect(w, { x: { b0: 1, b1: 5 }, y: { a0: 2, a1: 20 }, fill: [1, 0, 0, 1], fx: 1 });
    const p = rectPipeline();
    b.draw(p, batch(p, w), row, 0, 1);
    const style = calls.find((c) => c.op === "fill")!.state.fillStyle as string;
    const [r, g] = style.match(/\d+/g)!.map(Number);
    expect(r).toBeLessThan(255 * 0.55);
    expect(g).toBeGreaterThan(0);
  });

  it("a loading track's clips are desaturated, at the lane's opacity alone (#260)", () => {
    const { b, calls } = backend();
    const w = new InstanceWriter(RECT_STRIDE);
    writeRect(w, { x: { b0: 1, b1: 5 }, y: { a0: 2, a1: 20 }, fill: [1, 0, 0, 1], fx: Fx.Loading });
    writeRect(w, { x: { b0: 1, b1: 5 }, y: { a0: 2, a1: 20 }, fill: [1, 0, 0, 1], fx: Fx.Loading, alpha: 0.6 });
    const p = rectPipeline();
    b.draw(p, batch(p, w), row, 0, 2);
    const [full, faded] = calls.filter((c) => c.op === "fill").map((c) => (c.state.fillStyle as string).match(/\d+/g)!.map(Number));
    // Desaturated (green comes in), not faded by the effect itself...
    expect(full![1]).toBeGreaterThan(0);
    expect(full![0]).toBeCloseTo(255 * (0.213 + 0.787 * 0.2), -1);
    // ...but by the lane's opacity (mixed into the black background).
    expect(faded![0]).toBeCloseTo(full![0]! * 0.6, -1);
  });

  it("notes are rects in the clip body", () => {
    const { b, calls } = backend();
    const w = new InstanceWriter(NOTE_STRIDE);
    writeNote(w, 1, 2, 1, 2, { b0: 1, b1: 5, minW: 8 }, 0, [1, 1, 1, 1], 1);
    const p = notePipeline();
    b.draw(p, batch(p, w), row, 0, 1);
    const top = row.y + CLIP_INSET + CLIP_TITLE;
    expect(calls.find((c) => c.op === "fillRect")!.args).toEqual([20, top, 19, 6]);
  });

  it("text: atlas sprites tinted in their ink, cut to the clip box", () => {
    // Tinted sprites go to small offscreen canvases.
    let tinted: ImageData | null = null;
    const g0 = globalThis as unknown as { OffscreenCanvas?: unknown };
    const saved = g0.OffscreenCanvas;
    g0.OffscreenCanvas = class {
      constructor(
        readonly width: number,
        readonly height: number,
      ) {}
      getContext() {
        return {
          createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4), width: w, height: h }),
          putImageData: (img: ImageData) => (tinted = img),
        };
      }
    };
    const atlas = new GlyphAtlas(new FakeRaster(), { width: 256, height: 64 });
    const { b, calls } = backend(atlas);
    const font: FontSpec = { size: 10, weight: 500 };
    const g = atlas.sprite("abcdef", font)!;
    const w = new InstanceWriter(TEXT_STRIDE);
    // 36 px of text in a 30 px box (beats 0..1.5 at 20 px per beat).
    writeGlyph(w, { beat: 0, px: 0, y: 4, width: g.width, height: g.height, uv: g.uv, mode: 0, color: [1, 1, 1, 0.8], clip: { b0: 0, b1: 1.5 } });
    const p = textPipeline();
    b.draw(p, batch(p, w), row, 0, 1);
    const draw = calls.find((c) => c.op === "drawImage")!;
    expect(draw.args.slice(1)).toEqual([0, 0, 30, 12, 0, 14, 30, 12]);
    expect(draw.state.globalAlpha).toBeCloseTo(0.8, 5);
    // White ink, the atlas coverage as alpha.
    expect([...tinted!.data.slice(0, 4)]).toEqual([255, 255, 255, 200]);
    g0.OffscreenCanvas = saved;
  });

  it("skips pipelines it has no painter for", () => {
    const { b } = backend();
    const p = new Pipeline("mystery", "", "", [{ name: "i_a", size: 4 }]);
    const out = new Batch(p);
    out.set(new Float32Array(4));
    const warn = console.warn;
    console.warn = () => {};
    expect(b.draw(p, out, row, 0, 1)).toBe(false);
    console.warn = warn;
  });
});
