/**
 * Clip title badges, after the loop icon: the extensions' (`@/ext` "clips.badge", e.g.
 * "loading", optionally with a soft hatch over the clip), "REV" on reversed audio clips
 * and "Missing" on clips whose sample can't be found (media-refs; the clip is
 * also hatched and outlined).
 *
 * The scene lays the clip name out after them (`titleBadgesWidth`); this layer draws them.
 */

import type { Clip, ClipId } from "@/generated";
import { useMediaRefs } from "@/features/media-refs";
import { contributions, type ClipBadgeProvider } from "@/ext";
import { CLIP_INSET, CLIP_DRAW_MIN_PX, CLIP_RADIUS } from "../../laneBodyGeometry";
import type { GlyphAtlas, FontSpec } from "../glyphAtlas";
import type { ArrangerLayer } from "../layers";
import { Fx, writeGlyph, writeRect, type FxCode, type XSpec } from "../pipelines";
import type { Pipeline } from "../glContext";
import { FadeMode, writeFade } from "./fadePipeline";
import { editColors } from "./colors";
import { drawOrder, visibleSpans } from "./fadeModel";
import { isSmall } from "../hitTest";

export const CLIP_BADGES_LAYER = "clip-badges";
/** Beats standing for "no clip above covers this side". */
const FAR = 1e7;

/** A badge: the app's two, or an extension's (its text). */
export type BadgeKind = "reversed" | "missing" | (string & {});

interface Badge {
  kind: BadgeKind;
  /** An extension's badge: its text, and a soft hatch over the clip. */
  text?: string;
  hatch?: boolean;
}

/** Bumped when an extension's badges may have changed (`index.ts`): the layer rebuilds. */
let extVersion = 0;
export function extensionBadgesChanged(): void {
  extVersion++;
}

/** `.eth-clip-reversed .eth-badge`: `--eth-fs-xs` bold, `0 --eth-space-xs` padding, line height 1.1, pill. */
const REV_FONT: FontSpec = { size: 10, weight: 700 };
const REV_HEIGHT = 11;
/** `.eth-clip-reversed` margin-right. */
const REV_MARGIN = 2;
/** `.eth-media-refs__clip-badge`: `--eth-fs-2xs` 600, icon 1em, gap 1 px, line height 12 px. */
const MISSING_FONT: FontSpec = { size: 9, weight: 600 };
const MISSING_HEIGHT = 12;
const MISSING_ICON = 9;
const MISSING_GAP = 1;
const BADGE_PAD = 2;
/** The title's flex gap and top padding. */
const GAP = 2;
const TITLE_PAD_TOP = 1;

/** Lucide `TriangleAlert` (24×24, stroke 2). */
const ALERT_PATHS = ["m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3", "M12 9v4", "M12 17h.01"];
const ALERT_STROKE = 2;

/** The badges a clip's title shows, in order. */
export function badgesOf(
  clip: Clip,
  missing: ReadonlySet<string> = useMediaRefs.getState().missing,
  providers: ReadonlyArray<ClipBadgeProvider> = contributions("clips.badge"),
): BadgeKind[] {
  return badgeList(clip, missing, providers).map((b) => b.kind);
}

function badgeList(clip: Clip, missing: ReadonlySet<string>, providers: ReadonlyArray<ClipBadgeProvider>): Badge[] {
  const out: Badge[] = [];
  let hidesMissing = false;
  for (const p of providers) {
    const b = p.badge(clip);
    if (!b) continue;
    out.push({ kind: b.id, text: b.text, hatch: b.hatch });
    hidesMissing ||= !!b.hidesMissing;
  }
  if (clip.content.type !== "Audio") return out;
  if (clip.content.reversed) out.push({ kind: "reversed" });
  if (!hidesMissing && missing.has(clip.content.media)) out.push({ kind: "missing" });
  return out;
}

interface Laid extends Badge {
  /** Px from the title's content start, its box width and height, and the room it takes. */
  x: number;
  width: number;
  height: number;
  advance: number;
}

function layoutBadges(atlas: GlyphAtlas, badges: ReadonlyArray<Badge>): Laid[] {
  let x = 0;
  return badges.map((badge) => {
    const { kind } = badge;
    const width =
      kind === "reversed"
        ? atlas.measure("REV", REV_FONT) + 2 * BADGE_PAD
        : badge.text !== undefined
          ? atlas.measure(badge.text, MISSING_FONT) + 2 * BADGE_PAD
          : MISSING_ICON + MISSING_GAP + atlas.measure("Missing", MISSING_FONT) + 2 * BADGE_PAD;
    const advance = width + (kind === "reversed" ? REV_MARGIN : 0) + GAP;
    const laid = { ...badge, x, width, height: kind === "reversed" ? REV_HEIGHT : MISSING_HEIGHT, advance };
    x += advance;
    return laid;
  });
}

/** The title room the clip's badges take before its name (px; 0 without badges). */
export function titleBadgesWidth(atlas: GlyphAtlas, clip: Clip): number {
  const badges = badgeList(clip, useMediaRefs.getState().missing, contributions("clips.badge"));
  if (badges.length === 0) return 0;
  return layoutBadges(atlas, badges).reduce((w, b) => w + b.advance, 0);
}

/** Where the title's content starts (px from the clip's left): after the loop icon. */
export function badgesStart(clip: Clip): number {
  return 2 + (clip.looping.enabled ? 10 + GAP : 0);
}

export interface ClipBadgesLayer extends ArrangerLayer {
  /** The badges last drawn on a clip (test API). */
  badges(id: ClipId): BadgeKind[];
}

/**
 * The badges layer. It reads the missing media (`useMediaRefs`) at build time: the caller
 * invalidates it, and the scene's titles, when they change (see `index.ts`).
 */
export function clipBadgesLayer(pipeline: Pipeline): ClipBadgesLayer {
  const drawn = new Map<ClipId, BadgeKind[]>();
  return {
    id: CLIP_BADGES_LAYER,
    z: "over-clips",
    pipelines: [pipeline],
    deps: (ctx) => [ctx.lane.staticVersion, ctx.lane.view, useMediaRefs.getState().missing, extVersion],
    build({ lane, input, window }, out) {
      const colors = editColors(input.palette);
      const order = drawOrder(lane.items, input.selected);
      const spans = visibleSpans(order);
      const missing = useMediaRefs.getState().missing;
      const providers = contributions("clips.badge");
      const frozen = lane.frozen;
      const view = { scrollBeats: 0, pxPerBeat: window.ppb };
      const m = out.atlas;
      order.forEach((it, k) => {
        const { clip, bounds } = it;
        if (it.ghost) return;
        const badges = badgeList(clip, missing, providers);
        const kinds = badges.map((b) => b.kind);
        if (kinds.length) drawn.set(clip.id, kinds);
        else drawn.delete(clip.id);
        if (kinds.length === 0) return;
        const fx: FxCode = frozen ? Fx.Frozen : clip.muted ? Fx.Muted : Fx.None;
        const alpha = it.dragging ? 0.85 : 1;
        const box: XSpec = { b0: bounds.start, b1: bounds.start + bounds.length, minW: CLIP_DRAW_MIN_PX };
        const clipBox = { x: box, y: { a0: CLIP_INSET, a1: -CLIP_INSET, f1: 1 }, radius: CLIP_RADIUS };
        const small = isSmall(it, view);
        // Missing media: the hatch and the dashed outline over the whole clip. An extension's
        // badge may ask for a soft hatch.
        const isMissing = kinds.includes("missing");
        if (isMissing || badges.some((b) => b.hatch)) {
          const w = out.custom(pipeline);
          for (const [v0, v1] of spans[k]!) {
            const vis: [number, number] = [Math.max(v0, -FAR), Math.min(v1, FAR)];
            const spec = { box: { b0: box.b0, b1: box.b1, minW: CLIP_DRAW_MIN_PX }, f0: bounds.start, f1: bounds.start, vis, fx, alpha };
            writeFade(w, { ...spec, mode: FadeMode.Hatch, fill: isMissing ? colors.hatch : colors.softHatch });
            if (isMissing) writeFade(w, { ...spec, mode: FadeMode.Outline, fill: colors.warn });
          }
        }
        if (small) return;
        // Badges hidden under a clip above are skipped (they sit at the clip's start).
        if (!spans[k]!.some(([v0, v1]) => v0 <= bounds.start && v1 > bounds.start)) return;
        const left = badgesStart(clip);
        for (const b of layoutBadges(m, badges)) {
          const x = left + b.x;
          const top = CLIP_INSET + TITLE_PAD_TOP + (lane.titleHeight - TITLE_PAD_TOP - b.height) / 2;
          const fill = b.kind === "reversed" ? colors.accent : b.text !== undefined ? colors.dim : colors.warn;
          writeRect(out.rects, {
            x: { b0: bounds.start, b1: bounds.start, p0: x, p1: x + b.width },
            y: { a0: top, a1: top + b.height },
            radius: b.height / 2,
            fill,
            clip: clipBox,
            fx,
            alpha,
          });
          const font = b.kind === "reversed" ? REV_FONT : MISSING_FONT;
          const baseline = m.baseline(top, b.height, font);
          let pen = x + BADGE_PAD;
          if (b.kind === "missing") {
            const icon = m.icon("triangle-alert", ALERT_PATHS, MISSING_ICON, ALERT_STROKE);
            const iy = top + (b.height - MISSING_ICON) / 2;
            writeGlyph(out.text, {
              beat: bounds.start,
              px: pen,
              y: iy,
              width: icon.width,
              height: icon.height,
              uv: icon.uv,
              mode: 1,
              color: colors.badgeText,
              fx,
              alpha,
              clip: box,
            });
            pen += MISSING_ICON + MISSING_GAP;
          }
          const run = m.layout(b.kind === "reversed" ? "REV" : (b.text ?? "Missing"), font, Infinity, colors.badgeText);
          for (const { glyph, x: gx } of run.glyphs) {
            if (glyph.width === 0) continue;
            writeGlyph(out.text, {
              beat: bounds.start,
              px: pen + gx + glyph.left,
              y: baseline + glyph.top,
              width: glyph.width,
              height: glyph.height,
              uv: glyph.uv,
              mode: 0,
              color: colors.badgeText,
              fx,
              alpha,
              clip: box,
            });
          }
        }
      });
    },
    badges: (id) => drawn.get(id) ?? [],
  };
}
