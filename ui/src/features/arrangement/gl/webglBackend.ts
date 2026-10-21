/**
 * The WebGL2 backend of the arranger renderer (see backend.ts): the instanced pipelines of
 * shaders.ts, with the shared uniforms (`u_view`, `u_canvas`, `u_row`, `u_clip`, `u_bg`) set
 * once per pipeline and frame, `u_row` and `u_clip` per lane.
 */

import { mainHeight, type Row } from "../layout";
import { CLIP_INSET, contentTop } from "../laneBodyGeometry";
import type { FrameSetup, RenderBackend } from "./backend";
import { GlContext, type Batch, type Pipeline } from "./glContext";
import type { GlyphAtlas } from "./glyphAtlas";
import type { Pipelines } from "./scene";
import type { WaveTiles } from "./waveTiles";

export class WebglBackend implements RenderBackend {
  readonly kind = "webgl" as const;
  readonly ctx: GlContext;
  private prepared = new Set<Pipeline>();
  private current: Pipeline | null = null;
  private frame: FrameSetup | null = null;

  constructor(
    private canvas: HTMLCanvasElement,
    private core: Pipelines,
    private atlas: GlyphAtlas,
    private tiles: WaveTiles,
  ) {
    this.ctx = new GlContext(canvas);
  }

  get ok(): boolean {
    return !!this.ctx.gl;
  }

  get ready(): boolean {
    return this.ctx.ready;
  }

  begin(f: FrameSetup): void {
    const gl = this.ctx.gl!;
    this.frame = f;
    this.prepared.clear();
    this.current = null;
    gl.viewport(0, 0, f.width, f.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  }

  private bind(p: Pipeline, row: Row): boolean {
    const gl = this.ctx.gl!;
    const f = this.frame!;
    if (this.current !== p) {
      if (!p.use(this.ctx)) return false;
      this.current = p;
      if (!this.prepared.has(p)) {
        this.prepared.add(p);
        const scrollHi = Math.fround(f.scrollBeats);
        const bg = f.palette.bg;
        gl.uniform4f(p.uniform(this.ctx, "u_view"), scrollHi, f.scrollBeats - scrollHi, f.pxPerBeat, f.originX);
        gl.uniform4f(p.uniform(this.ctx, "u_canvas"), f.cssWidth, f.cssHeight, f.dpr, f.originY);
        gl.uniform3f(p.uniform(this.ctx, "u_bg"), bg[0], bg[1], bg[2]);
        if (p === this.core.text) gl.uniform1i(p.uniform(this.ctx, "u_atlas"), 0);
        if (p === this.core.wave) {
          gl.uniform1i(p.uniform(this.ctx, "u_peaks"), 1);
          gl.uniform4f(p.uniform(this.ctx, "u_over"), ...f.palette.over);
        }
      }
    }
    // Sprites rasterized and peak rows assigned since the last draw (lanes build their view
    // data as they are drawn: a lane's new rows must be uploaded before its waveforms draw,
    // not at the next frame, which may never come).
    if (p === this.core.text) this.atlas.bind(this.ctx, 0);
    if (p === this.core.wave) this.tiles.bind(this.ctx, 1);
    gl.uniform2f(p.uniform(this.ctx, "u_row"), row.y, mainHeight(row));
    // Per lane: a compact lane has no title band over its clips' content (their fades).
    gl.uniform2f(p.uniform(this.ctx, "u_clip"), CLIP_INSET, contentTop(mainHeight(row)));
    return true;
  }

  draw(p: Pipeline, batch: Batch, row: Row, first: number, count: number): boolean {
    if (count <= 0 || !this.bind(p, row)) return false;
    batch.draw(this.ctx, first, count);
    return true;
  }

  cut(band: { top: number; bottom: number } | null): void {
    const gl = this.ctx.gl;
    const f = this.frame;
    if (!gl || !f) return;
    if (!band) {
      gl.disable(gl.SCISSOR_TEST);
      return;
    }
    // Device px, from the canvas bottom.
    const top = Math.max(0, Math.floor((band.top - f.originY) * f.dpr));
    const bottom = Math.min(f.height, Math.ceil((band.bottom - f.originY) * f.dpr));
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(0, f.height - Math.max(top, bottom), f.width, Math.max(0, bottom - top));
  }

  end(): void {
    this.ctx.gl?.bindVertexArray(null);
    this.ctx.gl?.disable(this.ctx.gl.SCISSOR_TEST);
    this.frame = null;
  }

  readPixels(x: number, y: number, w: number, h: number): Uint8Array | null {
    const gl = this.ctx.gl;
    if (!gl || !this.ctx.ready) return null;
    const px = new Uint8Array(w * h * 4);
    gl.readPixels(x, this.canvas.height - y - h, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
    return px;
  }

  frameContext() {
    return { gl: this.ctx.gl, ctx2d: null };
  }

  onRestore(fn: () => void): () => void {
    return this.ctx.onRestore(fn);
  }

  onLose(fn: () => void): () => void {
    return this.ctx.onLose(fn);
  }

  simulateLoss(): boolean {
    return this.ctx.simulateLoss();
  }

  dispose(): void {
    this.ctx.dispose();
  }
}
