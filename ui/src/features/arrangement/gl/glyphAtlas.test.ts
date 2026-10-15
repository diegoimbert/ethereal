// The text atlas: shelf packing and eviction, the sprite cache key, layout with ellipsis,
// device pixel ratio changes. The browser rasterizer is faked (6 css px per character).
import { describe, expect, it } from "vitest";
import { collapseSpaces, ELLIPSIS, GlyphAtlas, ShelfPacker, shelfClass } from "./glyphAtlas";
import type { Rgba } from "./palette";
import { canonicalInk, type FontSpec, type RasterRun, type TextRasterizer } from "./textRaster";

const FONT: FontSpec = { size: 10, weight: 500 };

class FakeRaster implements TextRasterizer {
  dpr = 1;
  draws: string[] = [];
  resets = 0;
  reset(dpr: number): void {
    this.dpr = dpr;
    this.resets++;
  }
  measure(text: string): number {
    return [...text].length * 6;
  }
  metrics(): { ascent: number; descent: number } {
    return { ascent: 10, descent: 2 };
  }
  draw(text: string, _font: FontSpec, ink: string): RasterRun {
    this.draws.push(`${ink}|${text}`);
    const width = Math.ceil(this.measure(text) * this.dpr) + 2;
    const height = Math.ceil(12 * this.dpr) + 2;
    return { data: new Uint8Array(width * height).fill(255), width, height, originX: 1, baseline: Math.ceil(10 * this.dpr) + 1, advance: this.measure(text) };
  }
  icon(_paths: ReadonlyArray<string>, px: number): Uint8Array {
    return new Uint8Array(px * px);
  }
}

const WHITE: Rgba = [1, 1, 1, 1];
const INK: Rgba = [0.1, 0.11, 0.13, 1];

describe("ShelfPacker", () => {
  it("packs sprites of a height class on shelves, then opens new shelves", () => {
    const p = new ShelfPacker(100, 40);
    const a = p.alloc(60, 14, "a", 0)!;
    const b = p.alloc(30, 13, "b", 0)!;
    const c = p.alloc(30, 14, "c", 0)!;
    expect([a.x, a.y, b.x, b.y]).toEqual([0, 0, 60, 0]);
    // No room left on the first shelf: a second one below it.
    expect([c.x, c.y]).toEqual([0, shelfClass(14)]);
    expect(shelfClass(14)).toBe(16);
    // A taller class gets its own shelf; past the height, nothing.
    expect(p.alloc(10, 30, "d", 0)).toBeNull();
    expect(p.shelves).toHaveLength(2);
  });

  it("evicts the least recently used shelves only, and reuses them", () => {
    const p = new ShelfPacker(100, 32);
    p.alloc(90, 14, "old", 1);
    p.alloc(90, 14, "new", 2);
    expect(p.alloc(90, 14, "next", 2)).toBeNull();
    // Shelves last used in epoch 2 stay.
    expect(p.evict(2)).toEqual(["old"]);
    expect(p.evict(2)).toEqual([]);
    const at = p.alloc(90, 14, "next", 2)!;
    expect(at.y).toBe(0);
    expect(p.shelves[0]!.keys).toEqual(["next"]);
  });
});

describe("canonicalInk", () => {
  it("keys inks as Skia quantizes them (3 bits per channel)", () => {
    expect(canonicalInk([1, 1, 1, 1])).toBe("#e0e0e0");
    expect(canonicalInk([0, 0, 0, 1])).toBe("#000000");
    // #1b1d22 and #1a1c3f share a key (#000020); alpha does not count.
    expect(canonicalInk([0x1b / 255, 0x1d / 255, 0x22 / 255, 1])).toBe(canonicalInk([0x1a / 255, 0x1c / 255, 0x3f / 255, 0.5]));
    expect(canonicalInk([0xe8 / 255, 0xe8 / 255, 0xe8 / 255, 1])).not.toBe(canonicalInk([0x1b / 255, 0x1d / 255, 0x22 / 255, 1]));
  });
});

describe("collapseSpaces", () => {
  it("collapses like white-space: nowrap", () => {
    expect(collapseSpaces("  Overdub   spaces\tand\nlines ")).toBe("Overdub spaces and lines");
    expect(collapseSpaces("Bass Line 2")).toBe("Bass Line 2");
  });
});

describe("GlyphAtlas", () => {
  it("rasterizes each run once per font and ink class", () => {
    const r = new FakeRaster();
    const atlas = new GlyphAtlas(r, { width: 256, height: 64 });
    const a = atlas.layout("Bass", FONT, Infinity, WHITE);
    const b = atlas.layout("Bass", FONT, Infinity, [0.95, 0.95, 0.95, 1]);
    expect(b.glyphs[0]!.glyph).toBe(a.glyphs[0]!.glyph);
    atlas.layout("Bass", FONT, Infinity, INK);
    atlas.layout("Bass", { size: 10, weight: 700 }, Infinity, INK);
    expect(r.draws).toEqual(["#e0e0e0|Bass", "#000020|Bass", "#000020|Bass"]);
    // One sprite per run: 1:1 at device pixels, the pen at its origin.
    const g = a.glyphs[0]!.glyph;
    expect(a.glyphs).toHaveLength(1);
    expect([g.width, g.height, g.left, g.top]).toEqual([26, 14, -1, -11]);
    expect(g.uv[2] - g.uv[0]).toBe(26);
    expect(a.width).toBe(24);
  });

  it("ellipsizes like text-overflow: ellipsis, the ellipsis a run of its own", () => {
    const atlas = new GlyphAtlas(new FakeRaster(), { width: 256, height: 64 });
    // 40 px: the ellipsis (6) leaves 34: five characters (30).
    const run = atlas.layout("abcdefghij", FONT, 40);
    expect(run.text).toBe(`abcde${ELLIPSIS}`);
    expect(run.glyphs.map((g) => g.x)).toEqual([0, 30]);
    expect(run.width).toBe(36);
    // A space before the cut stays (Chrome keeps it).
    expect(atlas.layout("abcd fghij", FONT, 40).text).toBe(`abcd ${ELLIPSIS}`);
    // Fits: whole.
    expect(atlas.layout("abcdef", FONT, 36).text).toBe("abcdef");
    // No room for a character: the ellipsis alone.
    expect(atlas.layout("abcdef", FONT, 8).text).toBe(ELLIPSIS);
    // Grapheme clusters are not split ("é" as e + combining accent).
    expect(atlas.fit("eéeeee", FONT, 25)).toBe(`eé${ELLIPSIS}`);
  });

  it("drops its sprites when the device pixel ratio changes", () => {
    const r = new FakeRaster();
    const atlas = new GlyphAtlas(r, { width: 256, height: 64 });
    const v = atlas.version;
    atlas.layout("Bass", FONT, Infinity);
    atlas.reset(2);
    expect(atlas.version).toBe(v + 1);
    expect(atlas.devicePixelRatio).toBe(2);
    expect(atlas.size).toBe(0);
    const g = atlas.layout("Bass", FONT, Infinity).glyphs[0]!.glyph;
    expect(r.draws).toHaveLength(2);
    // 50 device px wide: 25 css px.
    expect(g.uv[2] - g.uv[0]).toBe(50);
    expect(g.width).toBe(25);
  });

  it("evicts what the text no longer uses when full, and rebuilds the text when all is in use", () => {
    const r = new FakeRaster();
    // Two shelves of 16 texels: one 62 px run each.
    const atlas = new GlyphAtlas(r, { width: 64, height: 32 });
    const v0 = atlas.version;
    expect(atlas.layout("aaaaaaaaaa", FONT, Infinity).glyphs).toHaveLength(1);
    expect(atlas.layout("bbbbbbbbbb", FONT, Infinity).glyphs).toHaveLength(1);
    // Full, every shelf used since the last rebuild: nothing drawn, the text is rebuilt.
    expect(atlas.layout("cccccccccc", FONT, Infinity).glyphs).toHaveLength(0);
    expect(atlas.version).toBe(v0 + 1);
    // The rebuild uses "a" only: "b"'s shelf goes to "c", without another rebuild.
    expect(atlas.layout("aaaaaaaaaa", FONT, Infinity).glyphs).toHaveLength(1);
    expect(atlas.layout("cccccccccc", FONT, Infinity).glyphs).toHaveLength(1);
    expect(atlas.version).toBe(v0 + 1);
    expect(atlas.size).toBe(2);
    // Text in view that can't fit doesn't rebuild twice in a row.
    expect(atlas.layout("dddddddddd", FONT, Infinity).glyphs).toHaveLength(0);
    expect(atlas.version).toBe(v0 + 1);
    // "b" comes back rasterized again once there is room.
    expect(r.draws.filter((d) => d.endsWith("|bbbbbbbbbb"))).toHaveLength(1);
  });
});
