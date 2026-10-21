/**
 * CPU side of the WebGL arranger: per-lane instance data built from the project mirror, and
 * kept until its inputs change.
 *
 * Each clip lane (`LaneScene`) has two kinds of instance data:
 *
 * - **static**: clip boxes, title bands, glow, rim, hover, loop seams and note previews.
 *   Positioned in beats (see shaders.ts), so it doesn't depend on zoom, scroll or the lane's
 *   height: it is rebuilt only when the lane's clips, notes, selection, hover, drag preview
 *   or theme change (and only for lanes on screen: others catch up when scrolled to).
 * - **view**: text (names are truncated to the clip's width with an ellipsis) and
 *   waveforms (the peak level follows the zoom), built for the view plus a viewport of
 *   margin on each side at the settled zoom, and rebuilt when the view leaves that window,
 *   the zoom settles at a new value, or peak tiles arrive.
 *
 * Each kind comes in two groups, drawn in order: `base` (the lane's clips in start order),
 * then `raised` (selected and dragged clips, above the others). The
 * base static group has a start-sorted index for horizontal culling.
 */

import type { Clip, ClipId, MediaRef, Note, Project, Track, TrackId, WarpMarker } from "@/generated";
import { isFrozenTrack } from "@/features/freeze/inert";
import { clipsOfTrack, notesOfClip, warpMarkersOfClip } from "@/state";
import type { TempoMap } from "@/timeline";
import type { EngineTransport } from "@/transport";
import { clipSourceMapper } from "@/domain/warpMap";
import { dbToGain, drumRows, noteRects, pitchRange, playsDrumRack } from "../clipDraw";
import { contentSegments } from "../clipTime";
import { CLIP_INSET, CLIP_MIN_PX, CLIP_RADIUS, CLIP_TITLE, isCompactLane, titleHeight } from "../laneBodyGeometry";
import { laneItemsOf, previewTouches, type LaneItem } from "../laneItems";
import { SMALL_CLIP_PX } from "../smallClips";
import { peakLevel, TILE_PEAKS, type PeakCache } from "../peaks";
import type { DragPreview } from "../uiStore";
import { Batch, type Pipeline, type RenderContext } from "./glContext";
import type { FontSpec, GlyphAtlas } from "./glyphAtlas";
import type { HitPart } from "./hitTest";
import { isRaised } from "./hitTest";
import { clipColors, withAlpha, type Palette, type Rgba } from "./palette";
import {
  Fx,
  InstanceWriter,
  NOTE_STRIDE,
  RECT_STRIDE,
  RectFlag,
  TEXT_STRIDE,
  WAVE_STRIDE,
  writeGlyph,
  writeNote,
  writeRect,
  writeWave,
  type FxCode,
  type XSpec,
  type YSpec,
} from "./pipelines";
import type { WaveTiles } from "./waveTiles";

export interface Hover {
  clip: ClipId;
  part: HitPart;
  /** A layer hit's details when they are a string (e.g. which fade handle). */
  data?: string;
}

/** Everything the scene is built from (snapshots of the stores). */
export interface SceneInput {
  project: Project;
  preview: DragPreview | null;
  selected: ReadonlySet<ClipId>;
  hover: Hover | null;
  renaming: ClipId | null;
  palette: Palette;
  tempo: TempoMap;
  transportKind: EngineTransport["kind"];
}

/**
 * Where a lane's clips come from when it isn't a track's main lane (a layer's extra lanes,
 * e.g. take lanes or the comp's pieces, see layers.ts): `items` returns the same array
 * while nothing changed. Default: the track's own clips with the drag preview.
 */
export interface LaneSource {
  items(input: SceneInput): ReadonlyArray<LaneItem>;
  /**
   * Clips drawn in full at any width (take lanes, the comp):
   * no small-clip rules (names keep their ellipsis).
   */
  elements?: boolean;
}

/** The view a lane's view-dependent data is built for. */
export interface ViewWindow {
  /** Settled px per beat (text truncation, peak level). */
  ppb: number;
  from: number;
  to: number;
  dpr: number;
}

export interface Pipelines {
  rect: Pipeline;
  text: Pipeline;
  note: Pipeline;
  wave: Pipeline;
}

export interface Shared {
  pipelines: Pipelines;
  atlas: GlyphAtlas;
  tiles: WaveTiles;
  peaks: PeakCache;
  /** Bumped when peak tiles arrive. */
  peaksGeneration: () => number;
  /** Title items besides the name (`TitleReserve`), and a counter bumped when they change. */
  titleReserves: () => ReadonlyArray<TitleReserve>;
  titleVersion: () => number;
}

/**
 * Room an item of a clip's title band takes besides the name (`renderer.addTitleReserve`):
 * the name is laid out (and ellipsized) in what is left, as a flex row
 * does. `left` items sit after the loop icon, before the name (REV / Missing badges);
 * `right` items at the band's end, after the name (audio-to-MIDI status, editors chip). The
 * width includes the item's gap (`TITLE_GAP`); 0 = the clip doesn't show the item. Call
 * `renderer.invalidateTitles()` when a width may have changed.
 */
export interface TitleReserve {
  id: string;
  side: "left" | "right";
  width(clip: Clip, atlas: GlyphAtlas, input: SceneInput): number;
}

/** A clip title's layout (px from the clip's left edge): where the name goes, and its room. */
export interface TitleLayout {
  /** After the loop icon: where the left items start. */
  leftStart: number;
  /** The name's left edge (its text starts `NAME_PAD_X` further) and room for its text. */
  nameStart: number;
  nameRoom: number;
  /** Where the right items start (the band's end minus their reserve). */
  rightStart: number;
}

/** The title layout of `clip`, `width` px wide (≥ SMALL_CLIP_PX), with the reserves given. */
export function titleLayout(clip: Clip, width: number, atlas: GlyphAtlas, input: SceneInput, reserves: ReadonlyArray<TitleReserve>): TitleLayout {
  let left = 0;
  let right = 0;
  for (const r of reserves) {
    const w = r.width(clip, atlas, input);
    if (r.side === "left") left += w;
    else right += w;
  }
  const leftStart = TITLE_PAD_X + (clip.looping.enabled ? ICON + TITLE_GAP : 0);
  const nameStart = leftStart + left;
  const rightStart = width - TITLE_PAD_X - right;
  return { leftStart, nameStart, nameRoom: rightStart - nameStart - 2 * NAME_PAD_X, rightStart };
}

/** Top of a line `height` px tall centered in a title band `titleH` px tall (px from the row top). */
const titleLineTop = (titleH: number, height: number) => CLIP_INSET + TITLE_PAD_TOP + (titleH - TITLE_PAD_TOP - height) / 2;

/**
 * Where a clip's name is drawn (px from the clip box's left edge and the row top): its text
 * box (the line box of `LINE_HEIGHT` px, as the title band lays it out) and its room.
 * `titleH`: the title band's height (the whole clip in a compact lane: `titleHeight`).
 */
export function nameBox(layout: TitleLayout, titleH = CLIP_TITLE): { left: number; top: number; room: number; height: number } {
  return {
    left: layout.nameStart + NAME_PAD_X,
    top: titleLineTop(titleH, LINE_HEIGHT),
    room: layout.nameRoom,
    height: LINE_HEIGHT,
  };
}

/** Clip title text (`.eth-clip__title`: `--eth-fs-xs`, `--eth-fw-medium`). */
export const TITLE_FONT: FontSpec = { size: 12, weight: 500 };
/** Title band layout (`.eth-clip__title`): padding (top, sides), gap, line height, icon. */
const TITLE_PAD_TOP = 1;
const TITLE_PAD_X = 2;
const TITLE_GAP = 2;
const NAME_PAD_X = 2;
const LINE_HEIGHT = 12;
const ICON = 10;
const ICON_STROKE = 1.75;
const ICON_OPACITY = 0.85;
/** Small clips (SmallClipsLayer): names past this width, inset from the clip's edges. */
const SMALL_NAME_PX = 24;
const SMALL_NAME_INSET = 3;

const BOX_Y: YSpec = { a0: CLIP_INSET, a1: -CLIP_INSET, f1: 1 };
const TITLE_Y: YSpec = { a0: CLIP_INSET, a1: CLIP_INSET + CLIP_TITLE };
const BODY_Y: YSpec = { a0: CLIP_INSET + CLIP_TITLE, a1: -CLIP_INSET, f1: 1 };

const NO_NOTES: ReadonlyArray<Note> = [];

/** Instances of one pipeline in two draw groups. */
class Groups {
  base: Batch;
  raised: Batch;
  constructor(p: Pipeline) {
    this.base = new Batch(p);
    this.raised = new Batch(p);
  }
  set(base: InstanceWriter, raised: InstanceWriter): void {
    this.base.set(base.take());
    this.raised.set(raised.take());
  }
  get bytes(): number {
    return this.base.bytes + this.raised.bytes;
  }
  dispose(ctx: RenderContext): void {
    this.base.dispose(ctx);
    this.raised.dispose(ctx);
  }
}

/** Start-sorted index of a lane's base clips: per clip, its instance offsets. */
export interface CullIndex {
  starts: Float64Array;
  /** Running maximum of the clip ends (beats), for the first clip reaching a beat. */
  maxEnds: Float64Array;
  under: Int32Array;
  notes: Int32Array;
  over: Int32Array;
}

/** The instance range of the base clips overlapping `[from, to]` (beats). */
export function cullRange(ix: CullIndex, from: number, to: number): [number, number] {
  const n = ix.starts.length;
  // First clip whose running max end reaches `from`.
  let lo = 0;
  let hi = n;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (ix.maxEnds[m]! < from) lo = m + 1;
    else hi = m;
  }
  const first = lo;
  // Past the last clip starting at or before `to`.
  lo = first;
  hi = n;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (ix.starts[m]! <= to) lo = m + 1;
    else hi = m;
  }
  return [first, lo];
}

export class LaneScene {
  track: Track;
  items: LaneItem[] = [];
  private own: ReadonlyArray<Clip> = [];
  private ownIds: ReadonlySet<ClipId> = new Set();
  private preview: DragPreview | null = null;
  private selSet: ReadonlySet<ClipId> | null = null;
  private selKey = "";
  private hoverKey = "";
  private palette: Palette | null = null;
  private color = -1;
  /** Plays a render: the track is frozen or inside a frozen group (clips drawn as frozen). */
  frozen = false;
  private notesTable: object | null = null;
  private notesUsed: Array<ReadonlyArray<Note>> = [];
  private staticDirty = true;
  /** Opacity of all the lane's clips (`setAlpha`: e.g. dimmed take lanes). */
  private alphaMul = 1;
  /** The clips' title band height (`titleHeight` of the lane's height: all of a compact lane). */
  titleHeight = CLIP_TITLE;
  /** A compact lane: its clips are a title band alone (no notes, waveform or loop seams). */
  compact = false;
  /** Bumped at each static rebuild (the view data depends on it). */
  staticVersion = 0;

  readonly under: Groups;
  /**
   * Selection glows (selected clips are raised: one batch), drawn for every lane before any
   * lane's clips, unclipped, so a glow spreads past its lane under the neighbours' clips.
   */
  readonly glow: Batch;
  readonly notes: Groups;
  readonly over: Groups;
  readonly text: Groups;
  readonly waves: Groups;
  cull: CullIndex = emptyIndex();
  /** The base clips in cull-index order, and the raised ones (as last built). */
  baseItems: LaneItem[] = [];
  raisedItems: LaneItem[] = [];
  /** Peak texture rows the view data uses. */
  waveRows: number[] = [];
  private viewKey: unknown[] | null = null;
  view: ViewWindow | null = null;

  constructor(
    track: Track,
    readonly shared: Shared,
    readonly source: LaneSource | null = null,
  ) {
    this.track = track;
    const p = shared.pipelines;
    this.under = new Groups(p.rect);
    this.glow = new Batch(p.rect);
    this.notes = new Groups(p.note);
    this.over = new Groups(p.rect);
    this.text = new Groups(p.text);
    this.waves = new Groups(p.wave);
  }

  get id(): TrackId {
    return this.track.id;
  }

  /** GPU bytes of this lane's instances. */
  get bytes(): number {
    return this.under.bytes + this.glow.bytes + this.notes.bytes + this.over.bytes + this.text.bytes + this.waves.bytes;
  }

  /**
   * Brings the lane's clip list and static instances up to date. `laneHeight`: the lane's
   * height when known (a compact lane draws its clips differently); else the last one given.
   */
  sync(track: Track, input: SceneInput, laneHeight?: number): void {
    const { project } = input;
    let itemsChanged = false;
    if (laneHeight !== undefined) {
      const th = titleHeight(laneHeight);
      if (th !== this.titleHeight) {
        this.titleHeight = th;
        this.compact = isCompactLane(laneHeight);
        this.staticDirty = true;
      }
    }
    if (track !== this.track) {
      if (track.color !== this.track.color) this.staticDirty = true;
      this.track = track;
    }
    if (this.source) {
      const next = this.source.items(input);
      if (next !== this.items) {
        this.items = next as LaneItem[];
        this.staticDirty = true;
      }
    }
    const own = this.source ? this.own : clipsOfTrack(project, track.id);
    if (own !== this.own) {
      if (!sameItems(own, this.own)) {
        itemsChanged = true;
        this.ownIds = new Set(own.map((c) => c.id));
      }
      this.own = own;
    }
    const preview = !this.source && previewTouches(input.preview, track.id, this.ownIds) ? input.preview : null;
    if (preview !== this.preview) {
      itemsChanged = true;
      this.preview = preview;
    }
    if (!this.source && (itemsChanged || this.items.length === 0)) {
      const next = laneItemsOf(this.own, track.id, preview, (id) => project.clips[id]);
      if (!sameLaneItems(next, this.items)) {
        this.items = next;
        this.staticDirty = true;
      }
    }
    if (input.selected !== this.selSet) {
      this.selSet = input.selected;
      let key = "";
      for (const it of this.items) if (input.selected.has(it.clip.id)) key += it.clip.id + ",";
      if (key !== this.selKey) {
        this.selKey = key;
        this.staticDirty = true;
      }
    }
    const h = input.hover;
    const hoverKey = h && this.items.some((it) => it.clip.id === h.clip) ? `${h.clip}|${h.part}` : "";
    if (hoverKey !== this.hoverKey) {
      this.hoverKey = hoverKey;
      this.staticDirty = true;
    }
    const frozen = isFrozenTrack(project, track);
    if (input.palette !== this.palette || track.color !== this.color || frozen !== this.frozen) {
      this.palette = input.palette;
      this.color = track.color;
      this.frozen = frozen;
      this.staticDirty = true;
    }
    if (project.notes !== this.notesTable) {
      this.notesTable = project.notes;
      const changed = this.items.some(
        (it, i) => it.clip.content.type === "Midi" && !sameItems(notesOfClip(project, it.clip.id), this.notesUsed[i] ?? NO_NOTES),
      );
      if (changed) this.staticDirty = true;
    }
    if (this.staticDirty) this.buildStatic(input);
  }

  /** Brings the view data (text, waveforms) up to date for `win`. */
  syncView(input: SceneInput, win: ViewWindow, viewFrom: number, viewTo: number): void {
    const v = this.view;
    const covered = !!v && viewFrom >= v.from - 1e-9 && viewTo <= v.to + 1e-9;
    const key = [
      this.staticVersion,
      input.renaming,
      input.project.media,
      input.project.warp_markers,
      input.tempo,
      this.shared.atlas.version,
      this.shared.peaksGeneration(),
      this.shared.tiles.version,
      this.shared.titleVersion(),
      win.ppb,
      win.dpr,
    ];
    if (covered && this.viewKey && key.every((k, i) => k === this.viewKey![i])) return;
    this.viewKey = key;
    this.view = win;
    this.buildView(input, win);
  }

  /** Opacity of all the lane's clips (like a CSS `opacity` on their container). */
  setAlpha(alpha: number): void {
    if (alpha === this.alphaMul) return;
    this.alphaMul = alpha;
    this.staticDirty = true;
  }

  dispose(ctx: RenderContext): void {
    for (const g of [this.under, this.notes, this.over, this.text, this.waves]) g.dispose(ctx);
    this.glow.dispose(ctx);
  }

  private fxOf(it: LaneItem): FxCode {
    return this.frozen ? Fx.Frozen : it.clip.muted ? Fx.Muted : Fx.None;
  }

  private buildStatic(input: SceneInput): void {
    this.staticDirty = false;
    this.staticVersion++;
    const { project, palette } = input;
    const sel = input.selected;
    const light = palette.theme === "light";
    const glow = new InstanceWriter(RECT_STRIDE);
    const W = () => ({ under: new InstanceWriter(RECT_STRIDE), notes: new InstanceWriter(NOTE_STRIDE), over: new InstanceWriter(RECT_STRIDE), glow });
    const base = W();
    const raised = W();
    this.baseItems = this.items.filter((it) => !isRaised(it, sel));
    this.raisedItems = this.items.filter((it) => isRaised(it, sel));
    const n = this.baseItems.length;
    const ix: CullIndex = {
      starts: new Float64Array(n),
      maxEnds: new Float64Array(n),
      under: new Int32Array(n + 1),
      notes: new Int32Array(n + 1),
      over: new Int32Array(n + 1),
    };
    let k = 0;
    let maxEnd = -Infinity;
    this.notesUsed = [];
    this.items.forEach((it, i) => {
      const up = isRaised(it, sel);
      const out = up ? raised : base;
      if (!up) {
        ix.starts[k] = it.bounds.start;
        maxEnd = Math.max(maxEnd, it.bounds.start + it.bounds.length);
        ix.maxEnds[k] = maxEnd;
        ix.under[k] = out.under.count;
        ix.notes[k] = out.notes.count;
        ix.over[k] = out.over.count;
        k++;
      }
      const notes = it.clip.content.type === "Midi" ? notesOfClip(project, it.clip.id) : NO_NOTES;
      this.notesUsed[i] = notes;
      this.clipStatic(it, out, input, notes, light);
    });
    ix.under[n] = base.under.count;
    ix.notes[n] = base.notes.count;
    ix.over[n] = base.over.count;
    this.cull = ix;
    this.under.set(base.under, raised.under);
    this.glow.set(glow.take());
    this.notes.set(base.notes, raised.notes);
    this.over.set(base.over, raised.over);
  }

  private clipStatic(
    it: LaneItem,
    out: { under: InstanceWriter; notes: InstanceWriter; over: InstanceWriter; glow: InstanceWriter },
    input: SceneInput,
    notes: ReadonlyArray<Note>,
    light: boolean,
  ): void {
    const { clip, bounds } = it;
    const colors = clipColors(input.palette, clip.color ?? this.track.color);
    const selected = !it.ghost && input.selected.has(clip.id);
    const fx = this.fxOf(it);
    const alpha = (it.ghost ? 0.45 : it.dragging ? 0.85 : 1) * this.alphaMul;
    const box: XSpec = { b0: bounds.start, b1: bounds.start + bounds.length, minW: CLIP_MIN_PX };
    const clipBox = { x: box, y: BOX_Y, radius: CLIP_RADIUS };
    const hover = this.hoverKey && !it.ghost && this.hoverKey.startsWith(clip.id + "|") ? this.hoverKey.slice(clip.id.length + 1) : null;

    // Selection halo (two box-shadows; none in the light theme): its own batch, unclipped
    // (`glow`: drawn before every lane's clips, so it spreads past the lane under them).
    if (selected && !light && colors.glowOuter && colors.glowInner) {
      writeRect(out.glow, { x: box, y: BOX_Y, radius: CLIP_RADIUS, blur: 14, fill: colors.glowOuter, flags: RectFlag.NoClip, fx, alpha });
      writeRect(out.glow, { x: box, y: BOX_Y, radius: CLIP_RADIUS, blur: 6, fill: colors.glowInner, flags: RectFlag.NoClip, fx, alpha });
    }
    if (this.compact) {
      // A compact lane: the clip is drawn like a body alone (its fill, a thin border in the
      // clip's color), the name on it in that color (clipText); all of it is the grab zone.
      const fill = selected || hover === "header" ? colors.bodySelected : colors.body;
      writeRect(out.under, { x: box, y: BOX_Y, radius: CLIP_RADIUS, fill, flags: RectFlag.Snap, fx, alpha });
      if (!selected) writeRect(out.over, { x: box, y: BOX_Y, radius: CLIP_RADIUS, stroke: colors.strong, strokeWidth: 1, flags: RectFlag.Snap, fx, alpha });
    } else {
      // The box, then its title band (the clip's grab zone).
      writeRect(out.under, { x: box, y: BOX_Y, radius: CLIP_RADIUS, fill: selected ? colors.bodySelected : colors.body, flags: RectFlag.Snap, fx, alpha });
      // `.eth-clip__title:hover` comes after `.eth-clip--selected .eth-clip__title`: hover wins.
      const title = hover === "header" ? colors.titleHover : selected ? colors.titleSelected : colors.strong;
      writeRect(out.under, { x: box, y: TITLE_Y, fill: title, flags: RectFlag.Snap, clip: clipBox, fx, alpha });
    }

    // MIDI note preview (shaped by the bounds, e.g. mid-drag), loops unrolled.
    const looping = bounds.looping ?? clip.looping;
    const shaped = { ...clip, length: bounds.length, offset: bounds.offset, looping };
    if (clip.content.type === "Midi" && notes.length && !this.compact) {
      const noteBox = { b0: box.b0, b1: box.b1, minW: CLIP_MIN_PX };
      const rects = noteRects(shaped, bounds.start, notes, bounds.start, bounds.start + bounds.length);
      if (playsDrumRack(input.project, clip.track)) {
        // Drums: a row per used pitch over the whole body, taller hits (negative rows: drum
        // mode in the note pipelines).
        const drums = drumRows(notes);
        for (const r of rects) {
          const pos = drums.rows > 1 ? drums.index.get(r.pitch)! / (drums.rows - 1) : 0.5;
          writeNote(out.notes, r.t0, r.t1, pos, -drums.rows, noteBox, fx, colors.strong, r.muted ? 0.3 : 0.55 + 0.45 * r.velocity, alpha);
        }
      } else {
        const range = pitchRange(notes);
        const rows = range.hi - range.lo + 1;
        for (const r of rects) {
          const pos = (r.pitch - range.lo) / Math.max(1, rows - 1);
          writeNote(out.notes, r.t0, r.t1, pos, rows, noteBox, fx, colors.strong, r.muted ? 0.3 : 0.55 + 0.45 * r.velocity, alpha);
        }
      }
    }
    // Loop seams: where a looped clip wraps around to its loop start.
    if (clip.looping.enabled && !this.compact) {
      for (const seg of contentSegments(shaped, bounds.start)) {
        if (seg.t0 <= bounds.start || seg.c0 !== looping.start) continue;
        writeRect(out.over, { x: { b0: seg.t0, b1: seg.t0, p1: 1 }, y: BODY_Y, fill: input.palette.seam, flags: RectFlag.Snap, clip: clipBox, fx, alpha });
      }
    }
    // Selection rim (above the content).
    if (selected) {
      writeRect(out.over, { x: box, y: BOX_Y, radius: CLIP_RADIUS, stroke: colors.rim, strokeWidth: 1, flags: RectFlag.Snap, fx, alpha });
    }
    // Hovered edge handle.
    if (hover === "resize-start" || hover === "resize-end") {
      writeRect(out.over, {
        x: box,
        y: BOX_Y,
        fill: withAlpha(input.palette.selection, 0.35),
        flags: hover === "resize-start" ? RectFlag.HandleStart : RectFlag.HandleEnd,
        clip: clipBox,
        alpha,
      });
    }
  }

  private buildView(input: SceneInput, win: ViewWindow): void {
    const sel = input.selected;
    const text = { base: new InstanceWriter(TEXT_STRIDE), raised: new InstanceWriter(TEXT_STRIDE) };
    const waves = { base: new InstanceWriter(WAVE_STRIDE), raised: new InstanceWriter(WAVE_STRIDE) };
    const rows: number[] = [];
    for (const it of this.items) {
      const end = it.bounds.start + Math.max(it.bounds.length, CLIP_MIN_PX / win.ppb);
      if (end < win.from || it.bounds.start > win.to) continue;
      const group = isRaised(it, sel) ? "raised" : "base";
      this.clipText(it, text[group], input, win);
      if (it.clip.content.type === "Audio" && !this.compact) this.clipWave(it, waves[group], input, win, rows);
    }
    this.text.set(text.base, text.raised);
    this.waves.set(waves.base, waves.raised);
    this.waveRows = rows;
  }

  private clipText(it: LaneItem, out: InstanceWriter, input: SceneInput, win: ViewWindow): void {
    const { clip, bounds } = it;
    const atlas = this.shared.atlas;
    const width = Math.max(CLIP_MIN_PX, bounds.length * win.ppb);
    const fx = this.fxOf(it);
    const alpha = (it.ghost ? 0.45 : it.dragging ? 0.85 : 1) * this.alphaMul;
    // On the title band, the band's text color; a compact lane's clip (a body alone): the clip's.
    const color = this.compact ? clipColors(input.palette, clip.color ?? this.track.color).strong : input.palette.titleText;
    const box: XSpec = { b0: bounds.start, b1: bounds.start + bounds.length, minW: CLIP_MIN_PX };
    // Centered in the title band (the whole clip in a compact lane).
    const lineTop = titleLineTop(this.titleHeight, LINE_HEIGHT);
    const baseline = atlas.baseline(lineTop, LINE_HEIGHT, TITLE_FONT);
    if (width < SMALL_CLIP_PX && !this.source?.elements) {
      // Small clips: no icon, the name
      // only past 24 px, cut at 3 px from the edges without an ellipsis.
      if (!clip.name || input.renaming === clip.id || width <= SMALL_NAME_PX) return;
      const run = atlas.layout(clip.name, TITLE_FONT, Infinity, color);
      const clipX: XSpec = { b0: bounds.start, b1: bounds.start + bounds.length, p1: -SMALL_NAME_INSET };
      for (const { glyph, x: gx } of run.glyphs) {
        if (glyph.width === 0 || gx > width) continue;
        writeGlyph(out, {
          beat: bounds.start,
          px: SMALL_NAME_INSET + gx + glyph.left,
          y: baseline + glyph.top,
          width: glyph.width,
          height: glyph.height,
          uv: glyph.uv,
          mode: 0,
          color,
          fx,
          alpha,
          clip: clipX,
        });
      }
      return;
    }
    if (clip.looping.enabled) {
      const icon = atlas.repeatIcon(ICON, ICON_STROKE);
      const y = titleLineTop(this.titleHeight, ICON);
      writeGlyph(out, {
        beat: bounds.start,
        px: TITLE_PAD_X,
        y,
        width: icon.width,
        height: icon.height,
        uv: icon.uv,
        mode: 1,
        color: withAlpha(color, ICON_OPACITY),
        fx,
        alpha,
        clip: box,
      });
    }
    // Title items (badges before the name, statuses after it) take their room first.
    const layout = titleLayout(clip, width, atlas, input, this.shared.titleReserves());
    if (!clip.name || input.renaming === clip.id) return;
    const room = layout.nameRoom;
    if (room <= 0) return;
    const run = atlas.layout(clip.name, TITLE_FONT, room, color);
    const left = layout.nameStart + NAME_PAD_X;
    for (const { glyph, x: gx } of run.glyphs) {
      if (glyph.width === 0) continue;
      writeGlyph(out, {
        beat: bounds.start,
        px: left + gx + glyph.left,
        y: baseline + glyph.top,
        width: glyph.width,
        height: glyph.height,
        uv: glyph.uv,
        mode: 0,
        color,
        fx,
        alpha,
        clip: box,
      });
    }
  }

  private clipWave(it: LaneItem, out: InstanceWriter, input: SceneInput, win: ViewWindow, rows: number[]): void {
    const { clip, bounds } = it;
    if (clip.content.type !== "Audio") return;
    const media: MediaRef | undefined = input.project.media[clip.content.media];
    if (!media || media.frames <= 0) return;
    const markers: ReadonlyArray<WarpMarker> = warpMarkersOfClip(input.project, clip.id);
    const toSeconds = clipSourceMapper(clip.content, markers, input.tempo.bpmAt(bounds.start), bounds.offset, input.transportKind);
    const bpp = 1 / (win.ppb * win.dpr);
    const level = peakLevel(Math.abs(toSeconds(bpp) - toSeconds(0)) * media.sample_rate);
    const colors = clipColors(input.palette, clip.color ?? this.track.color);
    const fx = this.fxOf(it);
    const alpha = (it.ghost ? 0.45 : it.dragging ? 0.85 : 1) * this.alphaMul;
    const gain = dbToGain(clip.content.gain);
    const box = { b0: bounds.start, b1: bounds.start + bounds.length, minW: CLIP_MIN_PX };
    const shaped = { length: bounds.length, offset: bounds.offset, looping: bounds.looping ?? clip.looping };
    const reversed = clip.content.reversed;
    const frames = media.frames;
    const sr = media.sample_rate;
    const cuts = markers.map((m) => m.beat).sort((a, b) => a - b);
    const tileFrames = TILE_PEAKS * level;
    const emit = (t0: number, t1: number, f0: number, f1: number) => {
      // [t0, t1] ↔ frames [f0, f1] linearly (f1 < f0 when reversed); per peak tile.
      const lo = Math.max(0, Math.min(f0, f1));
      const hi = Math.min(frames, Math.max(f0, f1));
      if (!(hi > lo)) return;
      const at = (f: number) => t0 + ((f - f0) / (f1 - f0 || 1)) * (t1 - t0);
      for (let ti = Math.floor(lo / tileFrames); ti * tileFrames < hi; ti++) {
        const a = Math.max(lo, ti * tileFrames);
        const b = Math.min(hi, (ti + 1) * tileFrames);
        const found = this.tileFor(media, level, ti, (a + b) / 2);
        if (!found) continue;
        const { row, start, spp } = found;
        const fa = f1 >= f0 ? a : b;
        const fb = f1 >= f0 ? b : a;
        const ta = at(fa);
        const tb = at(fb);
        writeWave(
          out,
          Math.min(ta, tb),
          Math.max(ta, tb),
          ((ta <= tb ? fa : fb) - start) / spp,
          ((ta <= tb ? fb : fa) - start) / spp,
          row,
          gain,
          fx,
          alpha,
          box,
          colors.strong,
        );
        if (!rows.includes(row)) rows.push(row);
      }
    };
    const from = Math.max(bounds.start, win.from);
    const to = Math.min(bounds.start + bounds.length, win.to);
    for (const seg of contentSegments(shaped, bounds.start, from, to)) {
      // Linear between warp markers: split the segment at them.
      const c0 = seg.c0;
      const c1 = seg.c0 + (seg.t1 - seg.t0);
      const points = [c0, ...cuts.filter((c) => c > c0 && c < c1), c1];
      for (let i = 0; i + 1 < points.length; i++) {
        const ca = points[i]!;
        const cb = points[i + 1]!;
        let fa = toSeconds(ca) * sr;
        let fb = toSeconds(cb) * sr;
        if (reversed) [fa, fb] = [frames - fa, frames - fb];
        emit(seg.t0 + (ca - c0), seg.t0 + (cb - c0), fa, fb);
      }
    }
  }

  /**
   * The texture row of the peaks around frame `f` at `level` (tile `index`): the tile itself,
   * else (while it loads) a coarser level already loaded.
   */
  private tileFor(media: MediaRef, level: number, index: number, f: number): { row: number; start: number; spp: number } | null {
    const peaks = this.shared.peaks;
    const tile = peaks.tile(media.id, level, index);
    const pick = (t: NonNullable<typeof tile>) => {
      const row = this.shared.tiles.row(t);
      return row < 0 ? null : { row, start: t.start_frame, spp: Math.max(1, t.samples_per_peak) };
    };
    if (tile) return pick(tile);
    for (let l = level * 2, n = 0; n < 6; l *= 2, n++) {
      const t = peaks.peek(media.id, l, Math.floor(f / (TILE_PEAKS * l)));
      if (t) return pick(t);
    }
    return null;
  }
}

function emptyIndex(): CullIndex {
  return { starts: new Float64Array(0), maxEnds: new Float64Array(0), under: new Int32Array(1), notes: new Int32Array(1), over: new Int32Array(1) };
}

function sameItems<T>(a: ReadonlyArray<T>, b: ReadonlyArray<T>): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function sameLaneItems(a: ReadonlyArray<LaneItem>, b: ReadonlyArray<LaneItem>): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.clip !== y.clip || x.bounds !== y.bounds || x.dragging !== y.dragging || x.ghost !== y.ghost) return false;
  }
  return true;
}

export type { Rgba };
