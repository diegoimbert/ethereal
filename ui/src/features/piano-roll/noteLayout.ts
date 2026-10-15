/**
 * The note grid's canvas scene and hit testing (pure: tested without a GPU or a layout).
 *
 * Two layers, drawn by `RectRenderer` (`noteCanvas.ts`) on two canvases with the DOM overlays
 * (shaded outside region, clip starts, section, marker) between them, so the stacking of the
 * DOM version is kept:
 * - rows: the pitch rows' tints (black keys, scale tones), C / scale-root lines and the
 *   vertical grid lines;
 * - notes: loop ghosts, notes, then the selected notes (with their glow) on top.
 *
 * Colors replicate the CSS the DOM version had (rows, grid lines, `.eth-pr-note` and its
 * modifiers; the `color-mix(in srgb, …)`s are computed on the CPU with `mix` / `withAlpha`).
 *
 * Coordinates: grid-local CSS px (x from the grid's left, y from the top row); a layer drawn
 * on a canvas covering grid rows `top … top + height` is shifted up by `top`.
 */

import { getPitchClass, scaleTone } from "@/domain/scales";
import type { ClipId, MusicalScale, Note, NoteId } from "@/generated";
import { beatsToPx, type GridLine, type GridLineLevel, type TimelineViewport } from "@/timeline";
import { mix, withAlpha, type Rgba } from "@/features/arrangement/gl/palette";
import { isBlackKey, noteHitZone, PITCHES } from "./geometry";
import type { GhostNote } from "./multiClip";
import type { InstanceList } from "./noteCanvas";

type Tone = ReturnType<typeof scaleTone>;

// ---- Palette ----------------------------------------------------------------------------

/** Theme colors and sizes the grid is drawn with (read from the page by the component). */
export interface GridColors {
  light: boolean;
  /** Selection halos off (Settings > Appearance > Glow); default on. */
  noGlow?: boolean;
  accent: Rgba;
  accentDim: Rgba;
  accentGlow: Rgba;
  text: Rgba;
  bgInset: Rgba;
  scaleRow: Rgba;
  scaleRootRow: Rgba;
  scaleRootLine: Rgba;
  /** `--eth-mix-white` / `--eth-mix-black`. */
  white: Rgba;
  black: Rgba;
  /** `--eth-radius-xs`, `--eth-border-width`, the selected glow's blur radii (px). */
  radius: number;
  borderWidth: number;
  glowInner: number;
  glowOuter: number;
  /** `--eth-opacity-scale-out-note`. */
  scaleOutNote: number;
}

/** The derived colors (the CSS `color-mix`es), once per theme. */
export interface GridPalette extends GridColors {
  rowBlack: Rgba;
  rowIn: Rgba;
  rowRoot: Rgba;
  cLine: Rgba;
  line: Record<GridLineLevel, Rgba>;
}

export function gridPalette(c: GridColors): GridPalette {
  return {
    ...c,
    // Black-key rows, and out-of-scale rows with the highlight on.
    rowBlack: withAlpha(c.bgInset, 0.8),
    // Scale rows (highlight on).
    rowIn: withAlpha(c.scaleRow, 0.85),
    rowRoot: withAlpha(c.scaleRootRow, 0.85),
    // The octave (C) line.
    cLine: withAlpha(c.text, 0.14),
    // Grid lines: a faint tint of the text color.
    line: { sub: withAlpha(c.text, 0.05), beat: withAlpha(c.text, 0.09), bar: withAlpha(c.text, 0.18) },
  };
}

// ---- Geometry ---------------------------------------------------------------------------

/** Row index of each MIDI pitch (the nearest visible row with folded rows), per row list. */
const rowTables = new WeakMap<readonly number[], Int16Array>();

export function rowTable(rows: readonly number[]): Int16Array {
  let t = rowTables.get(rows);
  if (t) return t;
  t = new Int16Array(PITCHES);
  for (let p = 0; p < PITCHES; p++) {
    if (rows.length === PITCHES) {
      t[p] = PITCHES - 1 - p;
      continue;
    }
    // Same rule as `pitchRow`: the first row of least distance.
    let nearest = 0;
    for (let i = 1; i < rows.length; i++) if (Math.abs(rows[i]! - p) < Math.abs(rows[nearest]! - p)) nearest = i;
    t[p] = nearest;
  }
  rowTables.set(rows, t);
  return t;
}

const rowOf = (table: Int16Array, pitch: number) => table[Math.min(PITCHES - 1, Math.max(0, Math.round(pitch)))]!;

export interface GridGeometry {
  vp: TimelineViewport;
  keyH: number;
  rows: readonly number[];
}

/** A note's box in grid px, as the DOM drew it (at least 2 px wide). */
export interface NoteBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Minimum drawn (and hit) width of a note. */
const MIN_NOTE_W = 2;

export function noteBox(n: Note, g: GridGeometry, table = rowTable(g.rows)): NoteBox {
  return {
    x: beatsToPx(n.start, g.vp),
    y: rowOf(table, n.pitch) * g.keyH,
    w: Math.max(MIN_NOTE_W, n.duration * g.vp.pxPerBeat),
    h: g.keyH,
  };
}

export interface NoteHit {
  note: Note;
  /** The resize edges or the body (see `noteHitZone`). */
  zone: "start" | "end" | "body";
  /** The note's box (grid px). */
  box: NoteBox;
}

/**
 * The note under grid point (x, y): the topmost one as drawn (selected notes over the others,
 * later notes over earlier ones), with the zone hit. Null over empty space.
 */
export function hitNote(notes: ReadonlyArray<Note>, selected: ReadonlySet<NoteId>, x: number, y: number, g: GridGeometry): NoteHit | null {
  const table = rowTable(g.rows);
  const row = Math.floor(y / g.keyH);
  if (y < 0 || row >= g.rows.length) return null;
  const beats = x / g.vp.pxPerBeat + g.vp.scrollBeats;
  const slack = MIN_NOTE_W / g.vp.pxPerBeat;
  for (const pass of [true, false]) {
    for (let i = notes.length - 1; i >= 0; i--) {
      const n = notes[i]!;
      if (selected.has(n.id) !== pass) continue;
      // Cheap rejects first (row, then beats) before the box.
      if (rowOf(table, n.pitch) !== row || beats < n.start || beats > n.start + Math.max(n.duration, slack)) continue;
      const box = noteBox(n, g, table);
      if (x < box.x || x > box.x + box.w) continue;
      return { note: n, zone: noteHitZone(x - box.x, box.w), box };
    }
  }
  return null;
}

/**
 * The grid's cursor over a hit (as the DOM's CSS had it): edges resize, a body moves (Alt:
 * drags velocity, except in draw mode); empty space draws in draw mode.
 */
export function gridCursor(hit: Pick<NoteHit, "zone"> | null, alt: boolean, drawMode: boolean): string {
  if (!hit) return drawMode ? "crosshair" : "default";
  if (hit.zone !== "body") return "ew-resize";
  return alt && !drawMode ? "ns-resize" : "move";
}

// ---- Layers -----------------------------------------------------------------------------

/** What an instance is (recorded for tests and debugging only). */
export type SceneTag =
  | { kind: "row"; pitch: number; tone: Tone; black: boolean }
  | { kind: "row-line"; pitch: number; line: "c" | "root" }
  | { kind: "grid-line"; level: GridLineLevel; beats: number }
  | { kind: "ghost"; key: string }
  | { kind: "glow"; id: NoteId }
  | { kind: "velocity"; id: NoteId; selected: boolean; velocity: number }
  | { kind: "velocity-outline"; id: NoteId }
  | { kind: "note"; id: NoteId; pitch: number; tone: Tone; selected: boolean; muted: boolean; inactive: boolean };

/** The canvas's span of the grid: rows `top … top + height` px, `width` px wide. */
export interface LayerFrame {
  top: number;
  height: number;
  /** Horizontal culling width (Infinity: none). */
  width: number;
}

export interface RowsLayerInput extends GridGeometry, LayerFrame {
  scale: MusicalScale;
  highlight: boolean;
  lines: ReadonlyArray<GridLine>;
}

/** Pitch rows (tints, C and scale-root lines) and grid lines, culled to the frame. */
export function buildRowsLayer(l: RowsLayerInput, p: GridPalette, out: InstanceList, tags?: SceneTag[]): void {
  const { keyH, rows, top } = l;
  const w = Number.isFinite(l.width) ? l.width : 0;
  const tag = (t: SceneTag) => tags?.push(t);
  const first = Math.max(0, Math.floor(top / keyH));
  const last = Math.min(rows.length - 1, Math.ceil((top + l.height) / keyH) - 1);
  const bw = p.borderWidth;
  for (let i = first; i <= last; i++) {
    const pitch = rows[i]!;
    const y = i * keyH - top;
    const tone = scaleTone(pitch, l.scale, l.highlight);
    const black = isBlackKey(pitch);
    const fill = tone === "in" ? p.rowIn : tone === "root" ? p.rowRoot : tone === "out" || (tone === undefined && black) ? p.rowBlack : null;
    if (fill) {
      out.push(0, y, w, keyH, { fill });
      tag({ kind: "row", pitch, tone, black });
    }
    // The C border sits on the row's bottom px; a root's inset line just above it.
    const c = getPitchClass(pitch) === 0;
    if (c) {
      out.push(0, y + keyH - bw, w, bw, { fill: p.cLine });
      tag({ kind: "row-line", pitch, line: "c" });
    }
    if (tone === "root") {
      out.push(0, y + keyH - bw - (c ? bw : 0), w, bw, { fill: p.scaleRootLine });
      tag({ kind: "row-line", pitch, line: "root" });
    }
  }
  const h = Math.min(l.height, rows.length * keyH - top);
  for (const line of l.lines) {
    const x = Math.round(beatsToPx(line.beats, l.vp));
    if (x + bw < 0 || x > w) continue;
    out.push(x, 0, bw, h, { fill: p.line[line.level] });
    tag({ kind: "grid-line", level: line.level, beats: line.beats });
  }
}

export interface NotesLayerInput extends GridGeometry, LayerFrame {
  notes: ReadonlyArray<Note>;
  ghosts?: ReadonlyArray<GhostNote>;
  selected: ReadonlySet<NoteId>;
  /** Multi-clip editing: notes of clips other than `focus` are dimmed. */
  multi: boolean;
  focus: ClipId;
  /** Note color by clip (multi-clip editing), else the accent. */
  clipColors?: ReadonlyMap<ClipId, Rgba>;
  scale: MusicalScale;
  highlight: boolean;
}

/** Opacity factor of the notes of non-focused clips (multi-clip editing). */
export const INACTIVE_OPACITY = 0.6;

/** A note's colors by state (velocity and dimming apart). */
interface Look {
  fill: Rgba;
  border: Rgba;
  top: Rgba;
  glowInner: Rgba | null;
  glowOuter: Rgba | null;
  /** The scale-out filter's opacity (1 otherwise). */
  opacity: number;
}

const looks = new WeakMap<GridPalette, Map<Rgba | null, Map<number, Look>>>();
const TONES: Tone[] = [undefined, "in", "out", "root"];

function lookOf(p: GridPalette, color: Rgba | null, selected: boolean, muted: boolean, tone: Tone): Look {
  let byColor = looks.get(p);
  if (!byColor) looks.set(p, (byColor = new Map()));
  let byFlags = byColor.get(color);
  if (!byFlags) byColor.set(color, (byFlags = new Map()));
  const key = (selected ? 1 : 0) | (muted ? 2 : 0) | (TONES.indexOf(tone) << 2);
  let look = byFlags.get(key);
  if (!look) byFlags.set(key, (look = computeLook(p, color, selected, muted, tone)));
  return look;
}

function computeLook(p: GridPalette, color: Rgba | null, selected: boolean, muted: boolean, tone: Tone): Look {
  const { accent, white, black, light } = p;
  let fill: Rgba;
  let border: Rgba;
  if (!color) {
    fill = !selected ? accent : light ? mix(accent, 0.75, black) : mix(accent, 0.7, white);
    border = !selected ? p.accentDim : light ? mix(accent, 0.45, black) : mix(accent, 0.3, white);
  } else {
    // Tinted by its clip (multi-clip editing).
    fill = !selected ? color : light ? mix(color, 0.7, black) : mix(color, 0.55, white);
    border = !selected ? mix(color, 0.6, black) : accent;
  }
  // A scale root's top border is the text color (except a tinted selected note's: accent).
  const top = tone === "root" && !(color && selected) ? p.text : border;
  const glow = selected && !light && !p.noGlow;
  const g = muted ? grayscale : (c: Rgba) => c;
  return {
    fill: g(fill),
    border: g(border),
    top: g(top),
    // Softer than the old CSS shadows: the GPU blur stacks both layers at full strength.
    glowInner: glow ? g(withAlpha(p.accentGlow, 0.6)) : null,
    glowOuter: glow ? g(withAlpha(p.accentGlow, 0.4)) : null,
    opacity: tone === "out" && !selected ? p.scaleOutNote : 1,
  };
}

/** CSS `filter: grayscale(1)` of a color. */
export function grayscale(c: Rgba): Rgba {
  const l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  return [l, l, l, c[3]];
}

/** The note opacity by velocity (and dimming of another clip's notes). */
export const noteOpacity = (velocity: number, inactive: boolean) => (0.45 + 0.55 * velocity) * (inactive ? INACTIVE_OPACITY : 1);

/** Ghosts, the selected notes' glows, notes, then the selected notes, culled to the frame.
 *  The glows go under every note body: a glow never covers a neighbouring note. */
export function buildNotesLayer(l: NotesLayerInput, p: GridPalette, out: InstanceList, tags?: SceneTag[]): void {
  const table = rowTable(l.rows);
  const { keyH, top, vp } = l;
  const bottom = top + l.height;
  const tag = (t: SceneTag) => tags?.push(t);
  const visible = (x: number, y: number, w: number) => y + keyH > top && y < bottom && x + w >= 0 && x <= l.width;
  const tones: Tone[] = new Array<Tone>(PITCHES);
  for (let pitch = 0; pitch < PITCHES; pitch++) tones[pitch] = scaleTone(pitch, l.scale, l.highlight);

  if (l.ghosts) {
    const dash = 3 * p.borderWidth;
    for (const ghost of l.ghosts) {
      const n = ghost.note;
      const x = beatsToPx(n.start, vp);
      const y = rowOf(table, n.pitch) * keyH;
      const w = Math.max(MIN_NOTE_W, n.duration * vp.pxPerBeat);
      if (!visible(x, y, w)) continue;
      const c = l.clipColors?.get(n.clip) ?? p.accent;
      out.push(x, y - top, w, keyH, { fill: withAlpha(c, 0.35), border: withAlpha(c, 0.7), borderWidth: p.borderWidth, radius: p.radius, dash });
      tag({ kind: "ghost", key: ghost.key });
    }
  }

  /** `part`: the note's glow (selected only), or its body. */
  const draw = (n: Note, selected: boolean, part: "glow" | "body" = "body") => {
    const x = beatsToPx(n.start, vp);
    const y = rowOf(table, n.pitch) * keyH;
    const w = Math.max(MIN_NOTE_W, n.duration * vp.pxPerBeat);
    if (!visible(x, y, w)) return;
    const tone = tones[Math.min(PITCHES - 1, Math.max(0, n.pitch))];
    const inactive = l.multi && n.clip !== l.focus;
    const look = lookOf(p, l.clipColors?.get(n.clip) ?? null, selected, n.muted, tone);
    const opacity = noteOpacity(n.velocity, inactive) * look.opacity;
    if (part === "glow") {
      if (look.glowOuter && look.glowInner) {
        out.push(x, y - top, w, keyH, { fill: look.glowOuter, radius: p.radius, blur: p.glowOuter, opacity });
        out.push(x, y - top, w, keyH, { fill: look.glowInner, radius: p.radius, blur: p.glowInner, opacity });
        tag({ kind: "glow", id: n.id });
        tag({ kind: "glow", id: n.id });
      }
      return;
    }
    out.push(x, y - top, w, keyH, { fill: look.fill, border: look.border, top: look.top, borderWidth: p.borderWidth, radius: p.radius, opacity });
    tag({ kind: "note", id: n.id, pitch: n.pitch, tone, selected, muted: n.muted, inactive });
  };
  const { notes, selected } = l;
  if (selected.size === 0) {
    for (const n of notes) draw(n, false);
    return;
  }
  for (const n of notes) if (selected.has(n.id)) draw(n, true, "glow");
  for (const n of notes) if (!selected.has(n.id)) draw(n, false);
  for (const n of notes) if (selected.has(n.id)) draw(n, true);
}

// ---- Velocity lane ----------------------------------------------------------------------

export interface VelocityLaneInput {
  notes: ReadonlyArray<Note>;
  selected: ReadonlySet<NoteId>;
  vp: TimelineViewport;
  /** The lane's height and drawn width (px). */
  height: number;
  width: number;
  /** A bar's width (`--eth-size-velocity-bar-width`) and the outline (`--eth-border-width`). */
  barWidth: number;
  borderWidth: number;
}

export interface VelocityColors {
  bar: Rgba;
  selected: Rgba;
  /** The selected bars' outline. */
  outline: Rgba;
}

/** A note's velocity bar: at its start (1 border width left of it), up from the bottom. */
export function velocityBar(n: Note, l: VelocityLaneInput): NoteBox {
  const h = Math.max(2, n.velocity * (l.height - 4));
  return { x: Math.round(beatsToPx(n.start, l.vp)) - l.borderWidth, y: l.height - h, w: l.barWidth, h };
}

/** One bar per visible note (later notes over earlier ones); selected bars outlined. */
export function buildVelocityLayer(l: VelocityLaneInput, c: VelocityColors, out: InstanceList, tags?: SceneTag[]): void {
  const o = l.borderWidth;
  for (const n of l.notes) {
    const b = velocityBar(n, l);
    if (b.x + b.w + o < 0 || b.x - o > l.width) continue;
    const selected = l.selected.has(n.id);
    // The outline is outside the box (CSS `outline`).
    if (selected) out.push(b.x - o, b.y - o, b.w + 2 * o, b.h + 2 * o, { fill: c.outline });
    out.push(b.x, b.y, b.w, b.h, { fill: selected ? c.selected : c.bar });
    if (selected) tags?.push({ kind: "velocity-outline", id: n.id });
    tags?.push({ kind: "velocity", id: n.id, selected, velocity: n.velocity });
  }
}

/** The note whose bar is under lane point (x, y) (the topmost), or null. */
export function hitVelocityBar(x: number, y: number, l: VelocityLaneInput): Note | null {
  for (let i = l.notes.length - 1; i >= 0; i--) {
    const n = l.notes[i]!;
    const b = velocityBar(n, l);
    if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= l.height) return n;
  }
  return null;
}

// ---- Inspection (tests) -------------------------------------------------------------------

/** One drawn instance and what it is. */
export type SceneItem = ReturnType<InstanceList["at"]> & SceneTag;

export interface GridScene {
  rows: SceneItem[];
  notes: SceneItem[];
}

/** Scene builders of the mounted canvases (by element), for tests and debugging. */
const probes = new WeakMap<Element, () => unknown>();

/**
 * Registers what `el` draws. Also reachable from the page as `el.ethScene()` (end-to-end
 * tests locate notes with it: they are not elements).
 */
export function registerScene(el: Element, build: () => GridScene | SceneItem[]): () => void {
  probes.set(el, build);
  const hook = el as Element & { ethScene?: () => unknown };
  hook.ethScene = build;
  return () => {
    if (probes.get(el) !== build) return;
    probes.delete(el);
    delete hook.ethScene;
  };
}

function probe(el: Element): unknown {
  const build = probes.get(el);
  if (!build) throw new Error("not a mounted piano-roll canvas");
  return build();
}

/** What the note grid `grid` (`data-testid="piano-roll-grid"`) draws now (grid px, top = 0). */
export const gridScene = (grid: Element) => probe(grid) as GridScene;

/** What the velocity lane `lane` (`data-testid="piano-roll-velocity"`) draws now (lane px). */
export const velocityScene = (lane: Element) => probe(lane) as SceneItem[];

/** The tagged instances of `list` (built with `tags`). */
export function tagged(list: InstanceList, tags: ReadonlyArray<SceneTag>): SceneItem[] {
  return tags.map((t, i) => ({ ...list.at(i), ...t }));
}
