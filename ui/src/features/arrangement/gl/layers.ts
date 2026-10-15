/**
 * Layers: how overlays plug into the WebGL arranger (fades, warp markers, comping, automation,
 * group summaries, presence, …). See README.md for the guide.
 *
 * A layer emits instances per lane with the same pipelines and lane-space coordinates as the
 * clips (`pipelines.ts`), cached until its `deps` change, and drawn at its `z`:
 *
 * - `under-clips`: above the lane background, below every clip;
 * - `over-clips`: above the lane's clips (base and raised), e.g. fade curves, warp markers;
 * - `top`: above everything of the lane, e.g. collaborator selections.
 *
 * `frame` draws once per frame after the lanes with raw GL (anything not per lane), and
 * `hitTest` lets a layer take the pointer before the clips.
 */

import type { Track } from "@/generated";
import type { Row } from "../layout";
import type { Batch, GL, Pipeline } from "./glContext";
import { Batch as BatchClass } from "./glContext";
import type { GlyphAtlas } from "./glyphAtlas";
import type { Hit } from "./hitTest";
import { InstanceWriter, NOTE_STRIDE, RECT_STRIDE, TEXT_STRIDE } from "./pipelines";
import type { LaneScene, LaneSource, SceneInput, ViewWindow } from "./scene";
import type { ArrangerRenderer } from "./renderer";

export type LayerZ = "under-clips" | "over-clips" | "top";

/** What a layer builds a lane from. */
export interface LaneLayerContext {
  /** The lane's clips (`lane.items`: drag previews applied, sorted by start). */
  lane: LaneScene;
  /** The lane's row (content px; `mainHeight(row)` is the clip lane's height). */
  row: Row;
  track: Track;
  input: SceneInput;
  /** The view window text and view-dependent data is built for (settled zoom). */
  window: ViewWindow;
}

/** Instance writers of a layer's lane build (see `writeRect` / `writeGlyph` / `writeNote`). */
export interface LayerOutput {
  rects: InstanceWriter;
  text: InstanceWriter;
  notes: InstanceWriter;
  atlas: GlyphAtlas;
  /**
   * Instances of a layer's own pipeline (`ArrangerLayer.pipelines`), drawn after `rects`
   * and before `notes` and `text`. The renderer sets the shared uniforms (`u_view`,
   * `u_canvas`, `u_row`, `u_clip`, `u_bg`: see `SHADER_COMMON` / `FRAG_COMMON`).
   */
  custom(p: Pipeline): InstanceWriter;
}

/**
 * A clip lane a layer adds to a row besides the track's main lane (take lanes, the comp's
 * pieces): drawn like the main lane's clips, with `top`/`height` (px from the row top) as
 * its lane box, after the main lane's clips and before the `over-clips` layers.
 */
export interface ExtraLane {
  /** Stable key of the lane (its scene is kept by this key while `source` is the same). */
  key: string;
  top: number;
  height: number;
  source: LaneSource;
  /** Opacity of all its clips (default 1). */
  alpha?: number;
}

/** What `frame()` draws with: `gl` on the WebGL backend, `ctx2d` on the Canvas2D one. */
export interface FrameContext {
  gl: GL | null;
  ctx2d: CanvasRenderingContext2D | null;
  renderer: ArrangerRenderer;
}

export interface ArrangerLayer {
  id: string;
  z: LayerZ;
  /**
   * Values the lane's instances depend on (compared with `===`); the build reruns when one
   * changes. Default: the lane's static version and the window its view data was built for
   * (rebuilt whenever the clips' instances are).
   */
  deps?(ctx: LaneLayerContext): ReadonlyArray<unknown>;
  /** Instances of one lane (only lanes on screen are built). */
  build?(ctx: LaneLayerContext, out: LayerOutput): void;
  /**
   * The pointer at lane px `x` (0 = the lane's left edge) and `y` (from the row top) of
   * `row`: a hit here wins over the clips.
   */
  hitTest?(x: number, y: number, row: Row): Hit | null;
  /** A press on this layer's hit (`hit.layer === id`): the layer handles the gesture. */
  onPointerDown?(hit: Hit, e: PointerEvent): void;
  /** A double-click on this layer's hit. */
  onDoubleClick?(hit: Hit, e: MouseEvent): void;
  /** A right-click on this layer's hit (e.g. the clip menu). */
  onContextMenu?(hit: Hit, e: MouseEvent): void;
  /** Pipelines of the layer's own (`out.custom(p)`), in their draw order. */
  pipelines?: ReadonlyArray<Pipeline>;
  /** Cursor over this layer's hit. */
  cursor?(hit: Hit): string;
  /**
   * Raw drawing once per frame, after the lanes: with `f.gl` on WebGL, `f.ctx2d` on the
   * Canvas2D fallback (handle both, or draw nothing on the one you don't).
   */
  frame?(f: FrameContext): void;
  /** Extra clip lanes of a row (see `ExtraLane`); asked every frame for rows on screen. */
  lanes?(ctx: LaneLayerContext): ReadonlyArray<ExtraLane>;
}

interface Built {
  deps: ReadonlyArray<unknown>;
  rects: Batch;
  text: Batch;
  notes: Batch;
  custom: Map<Pipeline, Batch>;
}

const STALE = Symbol("stale");
const NO_LANES: ExtraLane[] = [];

type DrawBatch = (p: Pipeline, b: Batch) => void;

export class LayerHost {
  private layers: ArrangerLayer[] = [];
  private built = new Map<ArrangerLayer, Map<string, Built>>();

  constructor(private renderer: ArrangerRenderer) {}

  /** Adds a layer (drawn after the layers added before it at the same z). Returns its removal. */
  add(layer: ArrangerLayer): () => void {
    this.layers.push(layer);
    this.built.set(layer, new Map());
    this.renderer.requestRender();
    return () => {
      this.layers = this.layers.filter((l) => l !== layer);
      this.built.delete(layer);
      this.renderer.requestRender();
    };
  }

  /** Rebuilds a layer's instances (all lanes) at the next frame. */
  invalidate(layer: ArrangerLayer): void {
    // Stale deps (rebuilt at the next draw, reusing the lanes' GPU buffers).
    for (const b of this.built.get(layer)?.values() ?? []) b.deps = [STALE];
    this.renderer.requestRender();
  }

  drawLane(z: LayerZ, ctx: LaneLayerContext, draw: DrawBatch): void {
    for (const layer of this.layers) {
      if (layer.z !== z || !layer.build) continue;
      const cache = this.built.get(layer)!;
      // Text instances hold atlas positions: a reset atlas (font loaded, dpr, …) rebuilds
      // every layer, whatever its own deps.
      const deps = [...(layer.deps?.(ctx) ?? [ctx.lane.staticVersion, ctx.lane.view]), this.renderer.atlas.version];
      let b = cache.get(ctx.track.id);
      if (!b || b.deps.length !== deps.length || b.deps.some((d, i) => d !== deps[i])) {
        const p = this.renderer.pipelines;
        b ??= { deps, rects: new BatchClass(p.rect), text: new BatchClass(p.text), notes: new BatchClass(p.note), custom: new Map() };
        b.deps = deps;
        const custom = new Map<Pipeline, InstanceWriter>();
        const out: LayerOutput = {
          rects: new InstanceWriter(RECT_STRIDE),
          text: new InstanceWriter(TEXT_STRIDE),
          notes: new InstanceWriter(NOTE_STRIDE),
          atlas: this.renderer.atlas,
          custom: (pipeline) => {
            let w = custom.get(pipeline);
            if (!w) custom.set(pipeline, (w = new InstanceWriter(pipeline.stride)));
            return w;
          },
        };
        layer.build(ctx, out);
        b.rects.set(out.rects.take());
        b.text.set(out.text.take());
        b.notes.set(out.notes.take());
        for (const pipeline of layer.pipelines ?? []) {
          let batch = b.custom.get(pipeline);
          if (!batch) b.custom.set(pipeline, (batch = new BatchClass(pipeline)));
          batch.set(custom.get(pipeline)?.take() ?? new Float32Array(0));
        }
        cache.set(ctx.track.id, b);
      }
      draw(b.rects.pipeline, b.rects);
      for (const batch of b.custom.values()) draw(batch.pipeline, batch);
      draw(b.notes.pipeline, b.notes);
      draw(b.text.pipeline, b.text);
    }
  }

  /** The extra clip lanes the layers add to a row (see `ExtraLane`). */
  extraLanes(ctx: LaneLayerContext): ExtraLane[] {
    let out: ExtraLane[] | null = null;
    for (const layer of this.layers) {
      const lanes = layer.lanes?.(ctx);
      if (lanes?.length) (out ??= []).push(...lanes);
    }
    return out ?? NO_LANES;
  }

  drawFrame(f: FrameContext): void {
    for (const layer of this.layers) layer.frame?.(f);
  }

  hitTest(x: number, y: number, row: Row): Hit | null {
    const lx = x - this.renderer.getViewport().headerWidth;
    for (let i = this.layers.length - 1; i >= 0; i--) {
      const layer = this.layers[i]!;
      const hit = layer.hitTest?.(lx, y - row.y, row);
      if (hit) return { ...hit, part: "layer", layer: layer.id };
    }
    return null;
  }

  byId(id: string): ArrangerLayer | undefined {
    return this.layers.find((l) => l.id === id);
  }

  dispose(): void {
    const ctx = this.renderer.ctx;
    for (const cache of this.built.values()) for (const b of cache.values()) [b.rects, b.text, b.notes, ...b.custom.values()].forEach((x) => x.dispose(ctx));
    this.built.clear();
    this.layers = [];
  }
}
