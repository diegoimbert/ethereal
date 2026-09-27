import { describe, expect, it } from "vitest";
import type { TrackId } from "@/generated";
import { clampToBox, FREE_SPACE_MIN_Y, screenToSong, scrollTopFor, songToScreen, viewportOf, visibleRowOf, type Lanes, type RowBox } from "./coords";

// A group "g" with children "a" and "b", then "c". Parent map for fold fallbacks.
const parents: Record<string, TrackId | null> = { g: null, a: "g", b: "g", c: null };
const parentOf = (id: TrackId) => parents[id];

/** Rows laid out top-down from `top` with the given heights (folded tracks omitted). */
function layout(top: number, rows: Array<[TrackId, number]>): RowBox[] {
  let y = top;
  return rows.map(([track, height]) => {
    const r = { track, top: y, height };
    y += height;
    return r;
  });
}

describe("song ↔ screen", () => {
  // Ada: zoom 20 px/beat, scrolled to beat 8, header 200 px, rows of 56 px.
  const adaLanes: Lanes = { left: 200, pxPerBeat: 20, scrollBeats: 8 };
  const adaRows = layout(30, [
    ["g", 56],
    ["a", 56],
    ["b", 56],
    ["c", 56],
  ]);
  // Bob: zoom 40 px/beat, scrolled to beat 0, header 150 px, taller rows, and "g" folded.
  const bobLanes: Lanes = { left: 150, pxPerBeat: 40, scrollBeats: 0 };
  const bobRows = layout(-20, [
    ["g", 100],
    ["c", 80],
  ]);

  it("maps a point to beats, a track and a 0..1 fraction of its row", () => {
    // 3/4 down "b" (top 142), 120 px into the lanes = 6 beats after beat 8.
    expect(screenToSong(320, 142 + 42, adaRows, adaLanes)).toEqual({ beats: 14, track: "b", y: 0.75 });
    // Over the ruler or below the last row: no track.
    expect(screenToSong(320, 10, adaRows, adaLanes)).toEqual({ beats: 14, track: null, y: 0 });
    expect(screenToSong(320, 1000, adaRows, adaLanes).track).toBeNull();
    // Over the header column: the left edge of the lanes.
    expect(screenToSong(50, 40, adaRows, adaLanes)).toMatchObject({ beats: 8, track: "g" });
  });

  it("maps a song position through another user's own layout", () => {
    const p = screenToSong(320, 30 + 3 * 56 + 14, adaRows, adaLanes); // "c" at 1/4, beat 14
    expect(p).toEqual({ beats: 14, track: "c", y: 0.25 });
    // Bob: beat 14 at 150 + 14 * 40; "c" is his second row (top 80, 80 px tall).
    expect(songToScreen(p, bobRows, bobLanes, parentOf, 5)).toEqual({ x: 150 + 560, y: 80 + 20, folded: false });
    // And back.
    expect(screenToSong(710, 100, bobRows, bobLanes)).toEqual(p);
  });

  it("maps free space below the tracks proportionally between users (ruler stays y 0)", () => {
    // Ada: tracks end at 400, her view ends at 600 (200 px free); Bob: 180 → 780 (600 px).
    const adaFree = { top: 400, bottom: 600 };
    const bobFree = { top: 180, bottom: 780 };
    const p = screenToSong(320, 450, adaRows, adaLanes, adaFree, 80);
    expect(p).toEqual({ beats: 14, track: null, y: 0.25 });
    expect(songToScreen(p, bobRows, bobLanes, parentOf, 5, bobFree, 80)).toEqual({ x: 150 + 560, y: 180 + 150, folded: false });
    // Right below the last track still reads as free space (not the ruler).
    expect(screenToSong(320, 400, adaRows, adaLanes, adaFree, 80).y).toBe(FREE_SPACE_MIN_Y);
    // Over the ruler (above the free space): y 0, drawn on the ruler.
    expect(screenToSong(320, 10, adaRows, adaLanes, adaFree, 80)).toEqual({ beats: 14, track: null, y: 0 });
    expect(songToScreen({ beats: 1, track: null, y: 0 }, bobRows, bobLanes, parentOf, 5, bobFree, 80)!.y).toBe(5);
    // Little or no visible free space: the fraction spans at least `minFree` (edge-clamped by the caller).
    expect(songToScreen({ beats: 1, track: null, y: 0.5 }, bobRows, bobLanes, parentOf, 5, { top: 900, bottom: 780 }, 80)!.y).toBe(940);
  });

  it("falls back to the folded group's row (middle) and hides unknown tracks", () => {
    const inB = { beats: 2, track: "b", y: 0.9 };
    expect(songToScreen(inB, bobRows, bobLanes, parentOf, 5)).toEqual({ x: 150 + 80, y: -20 + 50, folded: true });
    expect(songToScreen({ ...inB, track: "deleted" }, bobRows, bobLanes, parentOf, 5)).toBeNull();
    expect(songToScreen({ beats: 1, track: null, y: 0 }, bobRows, bobLanes, parentOf, 5)).toEqual({ x: 190, y: 5, folded: false });
    expect(visibleRowOf("a", adaRows, parentOf)).toEqual({ row: adaRows[1], folded: false });
  });

  it("clamps off-screen points to the visible edge", () => {
    const box = { x0: 150, y0: 30, x1: 800, y1: 500 };
    expect(clampToBox(400, 200, box)).toEqual({ x: 400, y: 200, edge: null });
    expect(clampToBox(-50, 200, box)).toEqual({ x: 150, y: 200, edge: "left" });
    expect(clampToBox(900, 600, box)).toEqual({ x: 800, y: 500, edge: "right" });
    expect(clampToBox(400, 0, box)).toEqual({ x: 400, y: 30, edge: "top" });
    expect(clampToBox(400, 900, box)).toEqual({ x: 400, y: 500, edge: "bottom" });
  });
});

describe("viewport (follow mode)", () => {
  // Content px: rows from 0.
  const leader = layout(0, [
    ["g", 56],
    ["a", 56],
    ["b", 56],
    ["c", 56],
  ]);
  const follower = layout(0, [
    ["g", 120],
    ["a", 30],
    ["b", 200],
    ["c", 56],
  ]);

  it("describes the top edge as a row and a fraction of it", () => {
    const v = viewportOf(leader, 56 * 2 + 14, { start: 4, end: 20 });
    expect(v).toEqual({ start: 4, end: 20, top_track: "b", top_offset: 0.25 });
    expect(viewportOf([], 0, { start: 0, end: 8 }).top_track).toBeNull();
  });

  it("puts the same row at the top with other row heights and folds", () => {
    const v = viewportOf(leader, 56 * 2 + 14, { start: 4, end: 20 });
    expect(scrollTopFor(v, follower, parentOf)).toBe(150 + 50);
    // "g" folded on the follower: its row goes to the top.
    const folded = layout(0, [
      ["g", 120],
      ["c", 56],
    ]);
    expect(scrollTopFor(v, folded, parentOf)).toBe(0);
    expect(scrollTopFor({ ...v, top_track: "c" }, folded, parentOf)).toBe(120 + 0.25 * 56);
    expect(scrollTopFor({ ...v, top_track: "gone" }, folded, parentOf)).toBeNull();
    expect(scrollTopFor({ ...v, top_track: null }, folded, parentOf)).toBe(0);
  });
});
