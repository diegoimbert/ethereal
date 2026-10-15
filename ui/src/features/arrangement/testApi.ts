/**
 * Test and dev API of the arranger, on `window.__ether.arrangement` (WebGL and Canvas2D): clip
 * geometry in client px, hit-testing, the clips on screen. The arranger draws no element
 * per clip, so the e2e helpers (apps/web/e2e/clips.ts) aim through this instead of
 * `[data-clip-id]` elements.
 */

import type { ClipId } from "@/generated";
import { useProjectStore } from "@/state";
import { CLIP_INSET, CLIP_TITLE } from "./laneBodyGeometry";
import { laneItems } from "./laneItems";
import { mainHeight, type Row } from "./layout";
import { glRenderer } from "./gl/active";
import { CLIP_BADGES_LAYER, CLIP_FADES_LAYER, type ClipBadgesLayer, type ClipFadesLayer } from "./gl/layers/index";
import { FADE_HANDLES, fadeGeometry, handleRect as fadeHandleRect, type FadeHandle } from "./gl/layers/fadeModel";
import type { DrawnView } from "./gl/renderer";
import { boxX, handleWidth, type HitPart } from "./gl/hitTest";
import { setRendererOverride, type RendererOverride } from "./gl/backendChoice";
import { arrangementView, useArrangementUi } from "./uiStore";
import { itemSelection } from "@/timeline";
import { laneTestApi, type LaneTestApi } from "./gl/layers/testApi";

export interface ClientRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type ClipPart = "box" | "header" | "body" | "resize-start" | "resize-end" | FadeHandle;

/** A clip's in-clip editing overlays as drawn. */
export interface ClipDecor {
  /** Fade curves drawn (the crossfade neighbours' included). */
  lines: number;
  crossfadeIn: boolean;
  crossfadeOut: boolean;
  /** Title badges ("reversed", "missing"). */
  badges: string[];
}

export interface ArrangementTestApi {
  /** The renderer drawing the clips now. */
  renderer(): "webgl" | "canvas2d" | null;
  /** A clip's box (or one of its parts) in client px, or null when it isn't laid out. */
  clipRect(id: ClipId): ClientRect | null;
  /** The text box of a clip's name (its line box; `width`: its room), or null. */
  nameRect(id: ClipId): ClientRect | null;
  handleRect(id: ClipId, part: ClipPart): ClientRect | null;
  /** The clip part under a client point (`{ clip, part }`), or null. */
  /**
   * Layer hits report their own part (e.g. `fade-in`), or `layer` with the layer and what it
   * hit (e.g. `{ layer: "automation", data: { point } }`).
   */
  hitTest(x: number, y: number): { clip: ClipId; part: HitPart | FadeHandle; layer?: string; data?: unknown } | null;
  /** Clips the renderer drew in the viewport (WebGL: at its last frame). */
  visibleClipIds(): ClipId[];
  /**
   * Clips the model says are in the viewport (what `visibleClipIds` must match), now or
   * (`"drawn"`, WebGL) for the view of the last frame drawn.
   */
  expectedVisibleClipIds(at?: "drawn"): ClipId[];
  /** WebGL: the view (scroll, zoom, size) the last frame was drawn for. */
  drawnView(): DrawnView | null;
  /** Pixels of a clip's body drawing (waveform or notes) that differ from its background. */
  bodyInk(id: ClipId): number;
  /** Waveform pixels marked past full scale; 0 when none. */
  overInk(id: ClipId): number;
  /** A clip's fade/crossfade overlay and badges, or null when the clip isn't drawn. */
  clipDecor(id: ClipId): ClipDecor | null;
  /** WebGL renderer statistics (frame times, draw calls, GPU memory), or null. */
  stats(): unknown;
  /** WebGL: rebuild the glyph atlas (as when the UI font loads). */
  resetText(): void;
  /** WebGL: lose and restore the context (tests of the restore path). */
  loseContext(): boolean;
  /** Clips drawn in the new-track ghost lanes of a drag below the last track. */
  newTrackGhostClipIds(): string[];
}

/** With the lane layers' part (automation points, comp pieces, take regions, group summaries). */
export type FullArrangementTestApi = ArrangementTestApi & LaneTestApi;

interface Plumbing {
  content: () => HTMLElement | null;
  scroller: () => HTMLElement | null;
  rows: () => ReadonlyArray<Row>;
}

function laneGeometry(p: Plumbing, id: ClipId, part: ClipPart): ClientRect | null {
  const project = useProjectStore.getState().project;
  const clip = project?.clips[id];
  const content = p.content();
  if (!project || !clip || !content) return null;
  const preview = useArrangementUi.getState().preview;
  const moved = preview && !preview.copy ? preview.bounds.get(id) : undefined;
  const track = moved?.track ?? clip.track;
  const row = p.rows().find((r) => r.track.id === track);
  if (!row) return null;
  const it = laneItems(project.clips, track, preview).find((i) => i.clip.id === id && !i.ghost);
  if (!it) return null;
  const v = arrangementView.getState();
  const hw = useArrangementUi.getState().headerWidth;
  const [x0, x1] = boxX(it, v);
  const box = content.getBoundingClientRect();
  const left = box.left + hw + x0;
  const top = box.top + row.y + CLIP_INSET;
  const height = mainHeight(row) - 2 * CLIP_INSET;
  const width = x1 - x0;
  const hwid = handleWidth(x0, x1);
  if (isFadeHandle(part)) {
    const g = fadeGeometry(it, v, mainHeight(row));
    const r = g && fadeHandleRect(g, part);
    return r ? { x: box.left + hw + r.x, y: box.top + row.y + r.y, width: r.width, height: r.height } : null;
  }
  switch (part) {
    case "header":
      return { x: left, y: top, width, height: CLIP_TITLE };
    case "body":
      return { x: left, y: top + CLIP_TITLE, width, height: height - CLIP_TITLE };
    case "resize-start":
      return { x: left, y: top, width: hwid, height };
    case "resize-end":
      return { x: left + width - hwid, y: top, width: hwid, height };
    default:
      return { x: left, y: top, width, height };
  }
}

/** Clips intersecting the scroll viewport, from the model. */
function expectedVisible(p: Plumbing, at?: DrawnView | null): ClipId[] {
  const project = useProjectStore.getState().project;
  const content = p.content();
  const scroller = p.scroller();
  if (!project || !content || !scroller) return [];
  const v = at ?? arrangementView.getState();
  const hw = useArrangementUi.getState().headerWidth;
  const width = at?.width ?? content.clientWidth - hw;
  const top = at?.scrollTop ?? scroller.scrollTop;
  const bottom = top + (at?.height ?? scroller.clientHeight);
  const preview = useArrangementUi.getState().preview;
  const out: ClipId[] = [];
  for (const row of p.rows()) {
    if (row.y >= bottom || row.y + mainHeight(row) <= top) continue;
    if (row.track.kind !== "Audio" && row.track.kind !== "Midi") continue;
    for (const it of laneItems(project.clips, row.track.id, preview)) {
      if (it.ghost) continue;
      const [x0, x1] = boxX(it, v);
      if (x1 > 0 && x0 < width) out.push(it.clip.id);
    }
  }
  return out;
}

function isFadeHandle(part: string): part is FadeHandle {
  return (FADE_HANDLES as ReadonlyArray<string>).includes(part);
}

export function installArrangementTestApi(p: Plumbing): () => void {
  if (typeof window === "undefined") return () => {};
  const w = window as unknown as { __ether?: Record<string, unknown> };
  const api: FullArrangementTestApi = {
    ...laneTestApi(p),
    renderer: () => glRenderer()?.backend ?? null,
    clipRect: (id) => laneGeometry(p, id, "box"),
    handleRect: (id, part) => laneGeometry(p, id, part),
    nameRect: (id) => {
      const r = glRenderer()?.nameRect(id);
      const content = p.content();
      if (!r || !content) return null;
      const box = content.getBoundingClientRect();
      return { x: box.left + r.x, y: box.top + r.y, width: r.width, height: r.height };
    },
    hitTest: (x, y) => {
      const r = glRenderer();
      const content = p.content();
      if (!r || !content) return null;
      const box = content.getBoundingClientRect();
      const hit = r.hitTest(x - box.left, y - box.top);
      if (hit?.part === "layer" && typeof hit.data === "string" && isFadeHandle(hit.data)) return { clip: hit.clip, part: hit.data };
      // Other layer hits (comp pieces, automation points) say which layer and what.
      return hit ? { clip: hit.clip, part: hit.part, ...(hit.layer ? { layer: hit.layer, data: hit.data } : {}) } : null;
    },
    visibleClipIds: () => {
      const r = glRenderer();
      return r ? r.visibleClipIds() : [];
    },
    expectedVisibleClipIds: (at) => expectedVisible(p, at === "drawn" ? glRenderer()?.drawnView : null),
    drawnView: () => glRenderer()?.drawnView ?? null,
    bodyInk: (id) => {
      const r = glRenderer();
      const rect = laneGeometry(p, id, "body");
      const content = p.content();
      if (!r || !rect || !content) return 0;
      const box = content.getBoundingClientRect();
      return r.inkIn({ x: rect.x - box.left, y: rect.y - box.top, width: rect.width, height: rect.height });
    },
    overInk: (id) => {
      const r = glRenderer();
      const rect = laneGeometry(p, id, "body");
      const content = p.content();
      const input = r?.getInput();
      if (!r || !rect || !content || !input) return 0;
      const box = content.getBoundingClientRect();
      return r.inkIn({ x: rect.x - box.left, y: rect.y - box.top, width: rect.width, height: rect.height }, input.palette.over);
    },
    clipDecor: (id) => {
      const r = glRenderer();
      if (!r) return null;
      const fades = r.layers.byId(CLIP_FADES_LAYER) as ClipFadesLayer | undefined;
      const badges = r.layers.byId(CLIP_BADGES_LAYER) as ClipBadgesLayer | undefined;
      const d = fades?.decor(id);
      const clip = useProjectStore.getState().project?.clips[id];
      if (!clip) return null;
      return { lines: d?.lines ?? 0, crossfadeIn: d?.crossfadeIn ?? false, crossfadeOut: d?.crossfadeOut ?? false, badges: badges?.badges(id) ?? [] };
    },
    stats: () => glRenderer()?.stats ?? null,
    loseContext: () => glRenderer()?.simulateContextLoss() ?? false,
    newTrackGhostClipIds: () => {
      return glRenderer()?.extraRowClipIds() ?? [];
    },
    resetText: () => glRenderer()?.resetText(),
  };
  w.__ether ??= {};
  w.__ether.arrangement = api;
  w.__ether.arrangementView = () => arrangementView;
  w.__ether.selectedClips = () => [...itemSelection.getState().selected.clip];
  // The hidden backend override (gl/backendChoice.ts): tests force the Canvas2D fallback.
  w.__ether.setArrangerRenderer = (r: RendererOverride | "webgl") => setRendererOverride(r);
  return () => {
    if (w.__ether?.arrangement === api) delete w.__ether.arrangement;
  };
}
