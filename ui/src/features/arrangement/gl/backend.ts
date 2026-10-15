/**
 * What the arranger renderer draws with: a backend that takes the scene's instance batches
 * (`rect`, `text`, `note`, `wave` and the layers' own pipelines) one lane at a time.
 *
 * - `WebglBackend` (`webglBackend.ts`): the instanced WebGL2 pipelines (shaders.ts).
 * - `Canvas2DBackend` (`canvas2d.ts`): the fallback without a working WebGL2. It reads the
 *   same instance data on the CPU and paints it with Canvas2D (rounded rects, shadows for
 *   the glow, sprites from the same text atlas, peaks as paths), so every layer works on it
 *   without code of its own.
 *
 * Scenes and layers never see the backend: they write instances; `ArrangerRenderer` hands
 * each batch to `draw` with the row it belongs to.
 */

import type { Row } from "../layout";
import type { BackendKind } from "./backendChoice";
import type { Batch, GL, Pipeline, RenderContext } from "./glContext";
import type { Palette } from "./palette";

/** The view and canvas of one frame. */
export interface FrameSetup {
  /** Canvas size (device px) and (css px). */
  width: number;
  height: number;
  cssWidth: number;
  cssHeight: number;
  dpr: number;
  scrollBeats: number;
  pxPerBeat: number;
  /** Content y of the canvas top (css px). */
  originY: number;
  /** Canvas x of the lanes' left edge (css px): > 0 when drawing under the header column. */
  originX: number;
  palette: Palette;
}

export interface RenderBackend {
  readonly kind: BackendKind;
  /** What batches and scenes dispose their GPU objects against (`gl` null on Canvas2D). */
  readonly ctx: RenderContext;
  /** The backend could start (WebGL: the context was created). */
  readonly ok: boolean;
  /** It can draw now (WebGL: the context isn't lost). */
  readonly ready: boolean;
  /** Starts a frame: clears the canvas. */
  begin(f: FrameSetup): void;
  /**
   * Draws instances `[first, first + count)` of `batch` (its pipeline's) in `row`'s lane
   * (the row's top and main height position them, see shaders.ts). False when nothing was
   * drawn (the pipeline isn't usable).
   */
  draw(p: Pipeline, batch: Batch, row: Row, first: number, count: number): boolean;
  end(): void;
  /**
   * RGBA bytes of a device-px rect of the canvas as last drawn (from its top-left), or
   * null. Row order isn't specified (tests count colors).
   */
  readPixels(x: number, y: number, w: number, h: number): Uint8Array | null;
  /** What a layer's `frame()` draws with: one of them is set. */
  frameContext(): { gl: GL | null; ctx2d: CanvasRenderingContext2D | null };
  /** Called when the context comes back after a loss (redraw). */
  onRestore(fn: () => void): () => void;
  /** Called when the context is lost. */
  onLose(fn: () => void): () => void;
  /** Loses and restores the context (tests); false when the backend can't. */
  simulateLoss(): boolean;
  dispose(): void;
}
