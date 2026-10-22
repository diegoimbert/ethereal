/**
 * The arranger renderer: one canvas over the clip lanes, redrawn only when something it
 * shows changed (a dirty flag and one `requestAnimationFrame`), from per-lane instance data
 * (`scene.ts`) culled to the view. Overlays plug in as layers (`layers.ts`, see README.md).
 * It draws with WebGL2, or the Canvas2D fallback without it (`backend.ts`).
 *
 * Geometry: the canvas covers the lanes (x from the header column's edge) and the scroll
 * viewport plus `MARGIN_Y` above and below, positioned in content px at the scroll position
 * it was drawn for. Vertical scrolling moves it with the content until the next frame
 * redraws it (no frame where clips lag behind the rows); horizontal pan and zoom are
 * uniforms (`u_view`), so they never rebuild instances.
 */

import type { ClipId, Project, Track, TrackId } from "@/generated";
import { settledZoom } from "@/timeline";
import { mainHeight, rowIndexAt, type Row } from "../layout";
import { CLIP_INSET, titleHeight } from "../laneBodyGeometry";
import type { PeakCache } from "../peaks";
import type { TimelineViewStore } from "@/timeline";
import type { RenderBackend } from "./backend";
import type { BackendKind } from "./backendChoice";
import { Canvas2DBackend } from "./canvas2d";
import type { Batch, Pipeline, RenderContext } from "./glContext";
import { WebglBackend } from "./webglBackend";
import { GlyphAtlas } from "./glyphAtlas";
import { boxX, handleWidth, hitLane, type Hit, type HitPart } from "./hitTest";
import { LayerHost, type ArrangerLayer, type LaneLayerContext } from "./layers";
import { notePipeline, rectPipeline, textPipeline, wavePipeline } from "./pipelines";
import { cullRange, LaneScene, nameBox, titleLayout, type LaneSource, type SceneInput, type Shared, type TitleReserve, type ViewWindow } from "./scene";
import { WaveTiles } from "./waveTiles";
import { renderDpr } from "@/timeline/renderDpr";

const NO_ROWS = (): ReadonlyArray<Row> => [];

/** Canvas rows drawn above and below the viewport (px): covers a frame of scrolling. */
export const MARGIN_Y = 96;
/** Glow and min-width reach past a clip's beats when culling (px). */
const CULL_PAD_PX = 24;
/** Frames an extra lane's scene is kept after it was last drawn. */
const EXTRA_LANE_TTL_FRAMES = 300;

export interface Viewport {
  /** Lane area width (css px): the content width minus the header column. */
  width: number;
  /** Scroll viewport height and position (content px). */
  height: number;
  scrollTop: number;
  /** Header column width (lanes start there, in content px). */
  headerWidth: number;
  /**
   * Css px drawn left of the lanes, under the header column (its translucent background shows
   * the clips passing under it); the canvas starts that much before the lanes. 0 = none.
   */
  bleed?: number;
}

/** The view a frame was drawn for. */
export interface DrawnView {
  scrollTop: number;
  height: number;
  width: number;
  scrollBeats: number;
  pxPerBeat: number;
}

export interface RendererStats {
  frames: number;
  /** Main-thread time of the last frames (ms), most recent last. */
  frameMs: number[];
  lanesDrawn: number;
  drawCalls: number;
  instances: number;
  /** GPU bytes: instance buffers, peak tiles, glyph atlas (Canvas2D: their CPU copies). */
  gpuBytes: { instances: number; waveTiles: number; atlas: number };
  /** The backend drawing. */
  backend: BackendKind;
}

export interface RendererDeps {
  canvas: HTMLCanvasElement;
  view: TimelineViewStore;
  peaks: PeakCache;
  /** On-screen rows (mid-animation positions included). */
  rows: () => ReadonlyArray<Row>;
  /** What draws (default WebGL; see backendChoice.ts). */
  backend?: BackendKind;
}

export class ArrangerRenderer {
  readonly backendImpl: RenderBackend;
  readonly atlas = new GlyphAtlas();
  readonly tiles = new WaveTiles();
  readonly layers = new LayerHost(this);
  readonly pipelines = { rect: rectPipeline(), text: textPipeline(), note: notePipeline(), wave: wavePipeline() };
  private shared: Shared;
  private lanes = new Map<TrackId, LaneScene>();
  /** Layers' extra lanes (`ExtraLane`) by key, with the frame they were last drawn at. */
  private extras = new Map<string, { scene: LaneScene; frame: number }>();
  private input: SceneInput | null = null;
  private viewport: Viewport = { width: 0, height: 0, scrollTop: 0, headerWidth: 0 };
  private scheduled = 0;
  /** An urgent draw is queued (see `requestRender`). */
  private microtask = false;
  private peaksGen = 0;
  private titleReserves: ReadonlyArray<TitleReserve> = [];
  private titleVersion = 0;
  private drawnIds = new Set<ClipId>();
  private drawnExtraIds = new Set<ClipId>();
  /** The view the last frame was drawn for (test API: what `visibleClipIds` reflects). */
  drawnView: DrawnView | null = null;
  private disposed = false;
  private extraRows: () => ReadonlyArray<Row> = NO_ROWS;
  private offs: Array<() => void> = [];
  /** Content y of the canvas top as last drawn. */
  originY = 0;
  stats: RendererStats;
  /** Called after each drawn frame. */
  onFrame: (() => void) | null = null;

  constructor(private deps: RendererDeps) {
    this.backendImpl =
      deps.backend === "canvas2d"
        ? new Canvas2DBackend(deps.canvas, this.atlas, this.tiles)
        : new WebglBackend(deps.canvas, this.pipelines, this.atlas, this.tiles);
    this.stats = {
      frames: 0,
      frameMs: [],
      lanesDrawn: 0,
      drawCalls: 0,
      instances: 0,
      gpuBytes: { instances: 0, waveTiles: 0, atlas: 0 },
      backend: this.backendImpl.kind,
    };
    this.shared = {
      pipelines: this.pipelines,
      atlas: this.atlas,
      tiles: this.tiles,
      peaks: deps.peaks,
      peaksGeneration: () => this.peaksGen,
      titleReserves: () => this.titleReserves,
      titleVersion: () => this.titleVersion,
    };
    this.offs.push(this.backendImpl.onRestore(() => this.requestRender()));
    this.offs.push(
      deps.peaks.subscribe(() => {
        this.peaksGen++;
        this.requestRender();
      }),
    );
  }

  /** The backend could start (WebGL: its context was created). */
  get ok(): boolean {
    return this.backendImpl.ok;
  }

  /** What draws: WebGL2 or the Canvas2D fallback. */
  get backend(): BackendKind {
    return this.backendImpl.kind;
  }

  /** The canvas drawn on. */
  get canvas(): HTMLCanvasElement {
    return this.deps.canvas;
  }

  /** What batches and scenes release their GPU objects against. */
  get ctx(): RenderContext {
    return this.backendImpl.ctx;
  }

  /** Called when the drawing context is lost (WebGL: it may come back). */
  onContextLost(fn: () => void): () => void {
    return this.backendImpl.onLose(fn);
  }

  setInput(input: SceneInput): void {
    this.input = input;
    this.requestRender();
  }

  getInput(): SceneInput | null {
    return this.input;
  }

  setViewport(v: Viewport): void {
    const p = this.viewport;
    if (p.width === v.width && p.height === v.height && p.scrollTop === v.scrollTop && p.headerWidth === v.headerWidth) return;
    this.viewport = v;
    this.requestRender(true);
  }

  getViewport(): Viewport {
    return this.viewport;
  }

  /**
   * Lanes drawn after the rows that aren't tracks (yet): `fn` returns rows with stand-in
   * tracks (the new-track ghost lanes of a clip drag). Not hit-tested. Returns its removal.
   */
  setExtraRows(fn: () => ReadonlyArray<Row>): () => void {
    this.extraRows = fn;
    this.requestRender();
    return () => {
      if (this.extraRows === fn) this.extraRows = NO_ROWS;
      this.requestRender();
    };
  }

  addLayer(layer: ArrangerLayer): () => void {
    return this.layers.add(layer);
  }

  /** Font loaded or device pixel ratio changed: text is rebuilt. */
  resetText(): void {
    this.atlas.reset();
    this.requestRender();
  }

  /**
   * Adds room for a title item besides the clip names (`TitleReserve`, scene.ts): names are
   * laid out around it. Returns its removal.
   */
  addTitleReserve(reserve: TitleReserve): () => void {
    this.titleReserves = [...this.titleReserves, reserve];
    this.invalidateTitles();
    return () => {
      this.titleReserves = this.titleReserves.filter((r) => r !== reserve);
      this.invalidateTitles();
    };
  }

  /** A title reserve's widths may have changed: names are laid out again. */
  invalidateTitles(): void {
    this.titleVersion++;
    this.requestRender();
  }

  /**
   * Marks the canvas dirty. Content changes draw at the next animation frame (several in
   * one frame draw once). View changes (`urgent`: pan, zoom, scroll) draw right after the
   * current task, in the same frame as the ruler and lane DOM they move with: a view
   * change made by another `requestAnimationFrame` callback after ours would otherwise
   * show a frame late.
   */
  requestRender(urgent = false): void {
    if (this.disposed) return;
    if (urgent) {
      if (this.microtask) return;
      this.microtask = true;
      queueMicrotask(() => {
        this.microtask = false;
        this.render();
      });
      return;
    }
    if (this.scheduled) return;
    this.scheduled = requestAnimationFrame(() => {
      this.scheduled = 0;
      this.render();
    });
  }

  /** Draws now. */
  render(): void {
    if (this.disposed || !this.input || !this.backendImpl.ready) return;
    if (this.scheduled) {
      cancelAnimationFrame(this.scheduled);
      this.scheduled = 0;
    }
    const t0 = performance.now();
    const { canvas } = this.deps;
    const v = this.viewport;
    const dpr = renderDpr();
    if (dpr !== this.atlas.devicePixelRatio) this.atlas.reset(dpr);
    const atlasVersion = this.atlas.version;
    const cssH = v.height + 2 * MARGIN_Y;
    const bleed = v.bleed ?? 0;
    const cssW = v.width + bleed;
    const w = Math.max(1, Math.round(cssW * dpr));
    const h = Math.max(1, Math.round(cssH * dpr));
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      canvas.style.width = `${cssW}px`;
      canvas.style.height = `${cssH}px`;
    }
    this.originY = v.scrollTop - MARGIN_Y;
    canvas.style.transform = `translateY(${this.originY}px)`;

    const view = this.deps.view.getState();
    const ppb = view.pxPerBeat;
    const scroll = view.scrollBeats;
    const viewFrom = scroll - (bleed + CULL_PAD_PX) / ppb;
    const viewTo = scroll + (v.width + CULL_PAD_PX) / ppb;
    const settled = settledZoom(this.deps.view);
    const span = v.width / settled;
    const win: ViewWindow = { ppb: settled, from: scroll - span, to: scroll + 2 * span, dpr };

    // A new frame of peak row use: the rows any lane draws in it are kept (see WaveTiles).
    this.tiles.tick();
    const backend = this.backendImpl;
    backend.begin({
      width: w,
      height: h,
      cssWidth: cssW,
      cssHeight: cssH,
      dpr,
      scrollBeats: scroll,
      pxPerBeat: ppb,
      originY: this.originY,
      originX: bleed,
      palette: this.input.palette,
    });
    let drawCalls = 0;
    let instances = 0;

    const rows = this.deps.rows();
    const top = this.originY;
    const bottom = this.originY + cssH;
    const project = this.input.project;
    const seen = new Set<TrackId>();
    this.drawnIds = new Set();
    this.drawnView = { scrollTop: v.scrollTop, height: v.height, width: v.width, scrollBeats: scroll, pxPerBeat: ppb };
    let lanesDrawn = 0;
    this.drawnExtraIds = new Set();
    const input = this.input;
    const drawRow = (shown: Row, track: Track, extraRow: boolean) => {
      // A row of a group folding / unfolding: drawn as laid out (its clips keep their look:
      // never compact on the way), cut at its height on screen, and without text.
      const folding = shown.folding;
      const row: Row = folding ? { ...folding, y: shown.y } : shown;
      if (folding) backend.cut({ top: shown.y, bottom: shown.y + shown.height });
      const lane = this.lane(track);
      seen.add(track.id);
      // Extra rows take no pointer (the ghost rows have `pointer-events: none`): no hover.
      lane.sync(track, extraRow && input.hover ? { ...input, hover: null } : input, mainHeight(row));
      lane.syncView(input, win, scroll, scroll + v.width / ppb);
      this.tiles.touch(lane.waveRows);
      const ctx: LaneLayerContext = { lane, row, track, input, window: win };
      lanesDrawn++;
      const draw = (p: Pipeline, batch: Batch, first?: number, n?: number) => {
        const count = n ?? batch.count - (first ?? 0);
        if (folding && p === this.pipelines.text) return;
        if (count <= 0 || !backend.draw(p, batch, row, first ?? 0, count)) return;
        drawCalls++;
        instances += count;
      };
      this.layers.drawLane("under-clips", ctx, (p, b) => draw(p, b));
      // Base clips, culled to the view.
      const [a, z] = cullRange(lane.cull, viewFrom, viewTo);
      const ix = lane.cull;
      if (z > a) {
        draw(this.pipelines.rect, lane.under.base, ix.under[a], ix.under[z]! - ix.under[a]!);
        draw(this.pipelines.wave, lane.waves.base);
        draw(this.pipelines.note, lane.notes.base, ix.notes[a], ix.notes[z]! - ix.notes[a]!);
        draw(this.pipelines.rect, lane.over.base, ix.over[a], ix.over[z]! - ix.over[a]!);
        draw(this.pipelines.text, lane.text.base);
      }
      // Selected and dragged clips above the others.
      draw(this.pipelines.rect, lane.under.raised);
      draw(this.pipelines.wave, lane.waves.raised);
      draw(this.pipelines.note, lane.notes.raised);
      draw(this.pipelines.rect, lane.over.raised);
      draw(this.pipelines.text, lane.text.raised);
      // Layers' extra clip lanes (take lanes, the comp's pieces), each in its own lane box.
      for (const extra of this.layers.extraLanes(ctx)) {
        const box: Row = { ...row, y: row.y + extra.top, laneHeight: extra.height, takesHeight: 0, height: extra.height };
        if (box.y >= bottom || box.y + box.height <= top) continue;
        const scene = this.extraLane(extra.key, track, extra.source);
        scene.setAlpha(extra.alpha ?? 1);
        scene.sync(track, input, extra.height);
        scene.syncView(input, win, scroll, scroll + v.width / ppb);
        this.tiles.touch(scene.waveRows);
        const drawIn = (p: Pipeline, b: Batch, first?: number, n?: number) => {
          const count = n ?? b.count - (first ?? 0);
          if (folding && p === this.pipelines.text) return;
          if (count <= 0 || !backend.draw(p, b, box, first ?? 0, count)) return;
          drawCalls++;
          instances += count;
        };
        const [ea, ez] = cullRange(scene.cull, viewFrom, viewTo);
        const ex = scene.cull;
        if (ez > ea) {
          drawIn(this.pipelines.rect, scene.under.base, ex.under[ea], ex.under[ez]! - ex.under[ea]!);
          drawIn(this.pipelines.wave, scene.waves.base);
          drawIn(this.pipelines.note, scene.notes.base, ex.notes[ea], ex.notes[ez]! - ex.notes[ea]!);
          drawIn(this.pipelines.rect, scene.over.base, ex.over[ea], ex.over[ez]! - ex.over[ea]!);
          drawIn(this.pipelines.text, scene.text.base);
        }
        drawIn(this.pipelines.rect, scene.under.raised);
        drawIn(this.pipelines.wave, scene.waves.raised);
        drawIn(this.pipelines.note, scene.notes.raised);
        drawIn(this.pipelines.rect, scene.over.raised);
        drawIn(this.pipelines.text, scene.text.raised);
      }
      this.layers.drawLane("over-clips", ctx, (p, b) => draw(p, b));
      this.layers.drawLane("top", ctx, (p, b) => draw(p, b));
      if (folding) backend.cut(null);
      // What is on screen (test API): the clips whose instances were drawn (the culled
      // base range and the raised ones) that intersect the viewport.
      const vis = { scrollBeats: scroll, pxPerBeat: ppb };
      if (shown.y < v.scrollTop + v.height && shown.y + mainHeight(shown) > v.scrollTop) {
        const issued = [...lane.baseItems.slice(a, z), ...lane.raisedItems];
        for (const it of issued) {
          if (it.ghost && !extraRow) continue;
          const [x0, x1] = boxX(it, vis);
          if (x1 > 0 && x0 < v.width) (extraRow ? this.drawnExtraIds : this.drawnIds).add(it.clip.id);
        }
      }
    };
    // The rows to draw: the tracks intersecting the canvas, then the lanes that aren't tracks
    // yet (a drag's new-track ghost lanes: their stand-in tracks).
    const visible: Array<[Row, Track, boolean]> = [];
    let i = Math.max(0, rowIndexAt(rows, Math.max(0, top)));
    for (; i < rows.length; i++) {
      const row = rows[i]!;
      if (row.y >= bottom) break;
      const track = project.tracks[row.track.id];
      if (!track) continue;
      // Layers draw in the whole row (take lanes, automation lanes), clips in its main lane.
      if (row.y + row.height <= top) continue;
      visible.push([row, track, false]);
    }
    const extra = this.extraRows();
    for (const row of extra) {
      if (row.y >= bottom || row.y + mainHeight(row) <= top) continue;
      visible.push([row, row.track, true]);
    }
    // Selection glows first, for every row: unclipped, they spread past their lane under the
    // neighbouring lanes' clips (drawn after).
    for (const [row, track, extraRow] of visible) {
      const lane = this.lane(track);
      lane.sync(track, extraRow && input.hover ? { ...input, hover: null } : input, mainHeight(row));
      if (lane.glow.count > 0 && backend.draw(this.pipelines.rect, lane.glow, row, 0, lane.glow.count)) {
        drawCalls++;
        instances += lane.glow.count;
      }
    }
    for (const [row, track, extraRow] of visible) drawRow(row, track, extraRow);
    this.layers.drawFrame({ ...backend.frameContext(), renderer: this });
    // Lanes of removed tracks.
    for (const [id, lane] of this.lanes) {
      if (!project.tracks[id] && !extra.some((r) => r.track.id === id)) {
        lane.dispose(this.ctx);
        this.lanes.delete(id);
      }
    }
    this.collectExtras();
    backend.end();

    const ms = performance.now() - t0;
    const s = this.stats;
    s.frames++;
    s.frameMs.push(ms);
    if (s.frameMs.length > 600) s.frameMs.splice(0, s.frameMs.length - 600);
    s.lanesDrawn = lanesDrawn;
    s.drawCalls = drawCalls;
    s.instances = instances;
    let bytes = 0;
    for (const lane of this.lanes.values()) bytes += lane.bytes;
    for (const e of this.extras.values()) bytes += e.scene.bytes;
    s.gpuBytes = { instances: bytes, waveTiles: this.tiles.bytes, atlas: this.atlas.bytes };
    s.backend = backend.kind;

    this.onFrame?.();
    // The atlas filled up during the frame and asked for the text to be rebuilt.
    if (this.atlas.version !== atlasVersion) this.requestRender();
  }

  /** The scene of a layer's extra lane (see `ExtraLane`), kept by key while it is drawn. */
  private extraLane(key: string, track: Track, source: LaneSource): LaneScene {
    let e = this.extras.get(key);
    if (e && e.scene.source !== source) {
      e.scene.dispose(this.ctx);
      e = undefined;
    }
    if (!e) this.extras.set(key, (e = { scene: new LaneScene(track, this.shared, source), frame: 0 }));
    e.frame = this.stats.frames;
    return e.scene;
  }

  /** Drops the extra lanes not drawn for a while (scrolled away, or gone). */
  private collectExtras(): void {
    for (const [key, e] of this.extras) {
      if (this.stats.frames - e.frame > EXTRA_LANE_TTL_FRAMES) {
        e.scene.dispose(this.ctx);
        this.extras.delete(key);
      }
    }
  }

  private lane(track: Track): LaneScene {
    let lane = this.lanes.get(track.id);
    if (!lane) this.lanes.set(track.id, (lane = new LaneScene(track, this.shared)));
    return lane;
  }

  /** A synced lane scene (for hit tests and the test API, also off screen). */
  laneOf(track: TrackId): LaneScene | null {
    const t = this.input?.project.tracks[track];
    if (!t || !this.input || (t.kind !== "Audio" && t.kind !== "Midi")) return null;
    const lane = this.lane(t);
    lane.sync(t, this.input);
    return lane;
  }

  /** The clip part at content px (`x` includes the header column). */
  hitTest(x: number, y: number): Hit | null {
    if (!this.input) return null;
    const rows = this.deps.rows();
    const row = rows[rowIndexAt(rows, y)];
    if (!row || x < this.viewport.headerWidth) return null;
    const layered = this.layers.hitTest(x, y, row);
    if (layered) return layered;
    const lane = this.laneOf(row.track.id);
    if (!lane) return null;
    const v = this.deps.view.getState();
    return hitLane(lane.items, this.input.selected, row.track.id, x - this.viewport.headerWidth, y - row.y, mainHeight(row), v);
  }

  /** A clip's box in content px (its drag preview position while dragged), or null. */
  clipRect(id: ClipId, part: HitPart | "box" = "box"): { x: number; y: number; width: number; height: number } | null {
    const project: Project | undefined = this.input?.project;
    const clip = project?.clips[id];
    if (!clip) return null;
    const preview = this.input?.preview?.bounds.get(id);
    const track = preview && !this.input?.preview?.copy ? preview.track : clip.track;
    const row = this.deps.rows().find((r) => r.track.id === track);
    const lane = this.laneOf(track);
    const it = lane?.items.find((i) => i.clip.id === id && !i.ghost);
    if (!row || !it) return null;
    const v = this.deps.view.getState();
    const [x0, x1] = boxX(it, v);
    const hw = this.viewport.headerWidth;
    const top = row.y + CLIP_INSET;
    const height = mainHeight(row) - 2 * CLIP_INSET;
    const hwid = handleWidth(x0, x1);
    const th = titleHeight(mainHeight(row));
    switch (part) {
      case "header":
      case "small":
        return { x: hw + x0, y: top, width: x1 - x0, height: th };
      case "body":
        return { x: hw + x0, y: top + th, width: x1 - x0, height: height - th };
      case "resize-start":
        return { x: hw + x0, y: top, width: hwid, height };
      case "resize-end":
        return { x: hw + x1 - hwid, y: top, width: hwid, height };
      default:
        return { x: hw + x0, y: top, width: x1 - x0, height };
    }
  }

  /**
   * The text box of a clip's name in content px (tests: the DOM reference of the text
   * spec): its line box, `width` its room. Null when the clip shows no name.
   */
  nameRect(id: ClipId): { x: number; y: number; width: number; height: number } | null {
    const box = this.clipRect(id);
    const clip = this.input?.project.clips[id];
    if (!box || !clip || !this.input) return null;
    const row = this.deps.rows().find((r) => r.track.id === clip.track);
    if (!row) return null;
    const n = nameBox(titleLayout(clip, box.width, this.atlas, this.input, this.titleReserves), titleHeight(mainHeight(row)));
    return { x: box.x + n.left, y: row.y + n.top, width: n.room, height: n.height };
  }

  /** Ids of the clips drawn in the viewport at the last frame. */
  visibleClipIds(): ClipId[] {
    return [...this.drawnIds];
  }

  /** Ids of the clips (copy ghosts included) drawn in the extra rows at the last frame. */
  extraRowClipIds(): ClipId[] {
    return [...this.drawnExtraIds];
  }

  /**
   * Pixels in a content-px rect that differ from its most common color (tests: a clip body's
   * waveform or notes), or (`match`) the pixels of that opaque color. Draws a frame first
   * and reads it back in the same task.
   */
  inkIn(rect: { x: number; y: number; width: number; height: number }, match?: readonly [number, number, number, number]): number {
    this.render();
    const dpr = renderDpr();
    const x = Math.max(0, Math.floor((rect.x - this.viewport.headerWidth + (this.viewport.bleed ?? 0)) * dpr));
    const y = Math.max(0, Math.floor((rect.y - this.originY) * dpr));
    const w = Math.min(this.deps.canvas.width - x, Math.ceil(rect.width * dpr));
    const h = Math.min(this.deps.canvas.height - y, Math.ceil(rect.height * dpr));
    if (w <= 0 || h <= 0) return 0;
    const px = this.backendImpl.readPixels(x, y, w, h);
    if (!px) return 0;
    if (match) {
      // Pixels of one (opaque) color, within a rounding tolerance.
      const want = match.map((c) => Math.round(c * 255));
      let n = 0;
      for (let i = 0; i < px.length; i += 4) {
        if (px[i + 3]! > 250 && Math.abs(px[i]! - want[0]!) <= 3 && Math.abs(px[i + 1]! - want[1]!) <= 3 && Math.abs(px[i + 2]! - want[2]!) <= 3) n++;
      }
      return n;
    }
    const counts = new Map<number, number>();
    for (let i = 0; i < px.length; i += 4) {
      const v = px[i + 3] === 0 ? -1 : (px[i]! << 16) | (px[i + 1]! << 8) | px[i + 2]!;
      counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    counts.delete(-1);
    const bg = [...counts].sort((a, b) => b[1] - a[1])[0]?.[0];
    let n = 0;
    for (const [v, c] of counts) if (v !== bg) n += c;
    return n;
  }

  /** The color (0–255 RGB, straight alpha) at content px (`x` includes the header column), or null. */
  pixelAt(x: number, y: number): [number, number, number] | null {
    this.render();
    const dpr = renderDpr();
    const px = Math.floor((x - this.viewport.headerWidth + (this.viewport.bleed ?? 0)) * dpr);
    const py = Math.floor((y - this.originY) * dpr);
    const { width, height } = this.deps.canvas;
    if (px < 0 || py < 0 || px >= width || py >= height) return null;
    const out = this.backendImpl.readPixels(px, py, 1, 1);
    if (!out) return null;
    // WebGL reads premultiplied alpha: un-premultiply.
    const a = this.backendImpl.kind === "webgl" ? out[3]! / 255 || 1 : 1;
    return [Math.round(out[0]! / a), Math.round(out[1]! / a), Math.round(out[2]! / a)];
  }

  /** Forces the context to be lost and restored (tests; WebGL only). */
  simulateContextLoss(): boolean {
    return this.backendImpl.simulateLoss();
  }

  dispose(): void {
    this.disposed = true;
    if (this.scheduled) cancelAnimationFrame(this.scheduled);
    for (const off of this.offs) off();
    for (const lane of this.lanes.values()) lane.dispose(this.ctx);
    this.lanes.clear();
    for (const e of this.extras.values()) e.scene.dispose(this.ctx);
    this.extras.clear();
    this.layers.dispose();
    this.backendImpl.dispose();
  }
}
