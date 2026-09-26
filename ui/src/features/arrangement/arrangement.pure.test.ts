import { describe, expect, it, vi } from "vitest";
import { AUTOMATION_BAR_HEIGHT, automationHeight } from "@/features/automation";
import type { Clip, Command, Note, PeakData, ReplyValue, Track } from "@/generated";
import { TempoMap } from "@/timeline";
import type { EngineTransport } from "@/transport";
import { actionForKey } from "./actions";
import { BROWSER_DRAG_MIME, readBrowserDrag } from "./browserDrop";
import { noteRects, pitchRange } from "./clipDraw";
import { contentBeatAt, contentSegments, mediaLengthInBeats, sourceSecondsMapper } from "./clipTime";
import {
  boundsCommand,
  dragPreview,
  duplicateCommand,
  moveCommand,
  splitCommand,
  toggleLoopCommand,
  type DragInput,
} from "./editMath";
import { barAround, groupSummaryKey } from "./helpers";
import { laneItems } from "./laneItems";
import { arrangementTracks, clipRects, HEADER_WIDTH, layoutRows, rowIndexAt, TRACK_HEIGHT } from "./layout";
import { PeakCache, peakLevel, peakRange, TILE_PEAKS } from "./peaks";
import { followScroll } from "./useFollowWithMargin";

// ── Fixtures ──────────────────────────────────────────────────────────────────────────

function track(id: string, kind: Track["kind"], order: string, parent: string | null = null): Track {
  return {
    id,
    kind,
    name: id,
    color: 0xff0000,
    order,
    parent,
    mixer: { volume: 0, pan: 0, mute: false, solo: false },
    input: { type: "None" },
    output: { type: "Default" },
    monitor: "Auto",
  };
}

function clip(id: string, trackId: string, start: number, length: number, extra: Partial<Clip> = {}): Clip {
  return {
    id,
    track: trackId,
    start,
    name: id,
    color: null,
    muted: false,
    length,
    offset: 0,
    looping: { enabled: false, start: 0, end: length },
    content: { type: "Midi" },
    ...extra,
  };
}

const audio = (id: string, trackId: string, start: number, length: number, extra: Partial<Clip> = {}): Clip =>
  clip(id, trackId, start, length, {
    content: {
      type: "Audio",
      media: "m1",
      gain: 0,
      transpose: 0,
      fade_in: 0,
      fade_out: 0,
      warp: { enabled: true, mode: "Complex", source_bpm: 120 },
    },
    ...extra,
  });

const tempo = TempoMap.constant(120);
const beatGrid = (b: number) => Math.round(b);
const tracks = [track("m1", "Midi", "a"), track("m2", "Midi", "b"), track("a1", "Audio", "c")];
const rows = layoutRows(tracks, new Set());

const commandsOf = (c: Command | null): Command[] =>
  !c ? [] : c.domain === "Edit" && c.command.type === "Batch" ? c.command.commands : [c];

// ── clipTime ──────────────────────────────────────────────────────────────────────────

describe("contentSegments", () => {
  it("maps an unlooped clip linearly from its offset", () => {
    expect(contentSegments({ length: 8, offset: 2, looping: { enabled: false, start: 0, end: 4 } }, 10)).toEqual([
      { t0: 10, t1: 18, c0: 2 },
    ]);
  });

  it("unrolls loops after the offset reaches the loop end", () => {
    const c = { length: 10, offset: 1, looping: { enabled: true, start: 0, end: 4 } };
    expect(contentSegments(c, 0)).toEqual([
      { t0: 0, t1: 3, c0: 1 },
      { t0: 3, t1: 7, c0: 0 },
      { t0: 7, t1: 10, c0: 0 },
    ]);
  });

  it("restricts to a range and skips whole repetitions", () => {
    const c = { length: 100, offset: 0, looping: { enabled: true, start: 0, end: 4 } };
    expect(contentSegments(c, 0, 41, 45)).toEqual([
      { t0: 41, t1: 44, c0: 1 },
      { t0: 44, t1: 45, c0: 0 },
    ]);
  });

  it("contentBeatAt follows loops", () => {
    const c = clip("c", "m1", 8, 16, { looping: { enabled: true, start: 0, end: 4 } });
    expect(contentBeatAt(c, 8)).toBe(0);
    expect(contentBeatAt(c, 13)).toBe(1);
    expect(contentBeatAt(c, 30)).toBeNull();
  });
});

describe("source mapping", () => {
  it("uses the source tempo, falling back to the song tempo", () => {
    expect(sourceSecondsMapper(120, 90, [])(4)).toBe(2);
    expect(sourceSecondsMapper(null, 60, [])(4)).toBe(4);
  });

  it("interpolates between warp markers and extends with the source tempo", () => {
    const f = sourceSecondsMapper(120, 120, [
      { beat: 4, source: 1 },
      { beat: 0, source: 0 },
    ]);
    expect(f(2)).toBeCloseTo(0.5);
    expect(f(6)).toBeCloseTo(2);
  });

  it("finds the media length in beats", () => {
    const f = sourceSecondsMapper(120, 120, []);
    expect(mediaLengthInBeats({ frames: 44100 * 8, sample_rate: 44100 }, f)).toBeCloseTo(16, 6);
  });
});

// ── layout ────────────────────────────────────────────────────────────────────────────

describe("layout", () => {
  const all = [
    track("g", "Group", "a"),
    track("c1", "Midi", "a", "g"),
    track("g2", "Group", "b", "g"),
    track("c2", "Audio", "a", "g2"),
    track("r", "Return", "c"),
    track("master", "Master", "d"),
    track("t", "Audio", "e"),
  ];
  // Display order as `tracksOrdered` gives it: depth-first.
  const ordered = [all[0]!, all[1]!, all[2]!, all[3]!, all[4]!, all[5]!, all[6]!];

  it("puts returns and master last and hides descendants of folded groups", () => {
    expect(arrangementTracks(ordered, new Set()).map((t) => t.id)).toEqual(["g", "c1", "g2", "c2", "t", "r", "master"]);
    expect(arrangementTracks(ordered, new Set(["g2"])).map((t) => t.id)).toEqual(["g", "c1", "g2", "t", "r", "master"]);
    expect(arrangementTracks(ordered, new Set(["g"])).map((t) => t.id)).toEqual(["g", "t", "r", "master"]);
  });

  it("stacks rows with depth and automation height", () => {
    const r = layoutRows(ordered, new Set(), (id) => (id === "c1" ? 30 : 0));
    expect(r.map((x) => [x.track.id, x.depth, x.y])).toEqual([
      ["g", 0, 0],
      ["c1", 1, TRACK_HEIGHT],
      ["g2", 1, 2 * TRACK_HEIGHT + 30],
      ["c2", 2, 3 * TRACK_HEIGHT + 30],
      ["t", 0, 4 * TRACK_HEIGHT + 30],
      ["r", 0, 5 * TRACK_HEIGHT + 30],
      ["master", 0, 6 * TRACK_HEIGHT + 30],
    ]);
    expect(rowIndexAt(r, -1)).toBe(-1);
    expect(rowIndexAt(r, TRACK_HEIGHT + 70)).toBe(1);
    expect(rowIndexAt(r, 10000)).toBe(r.length);
  });

  it("adds the automation slot height of ui-automation to every row", () => {
    const closed = { open: new Set<string>(), shown: {} };
    const r = layoutRows(ordered, new Set(), (id) => automationHeight(closed, id));
    expect(r[1]).toMatchObject({ y: TRACK_HEIGHT + AUTOMATION_BAR_HEIGHT, laneHeight: TRACK_HEIGHT, height: TRACK_HEIGHT + AUTOMATION_BAR_HEIGHT });
    expect(TRACK_HEIGHT + AUTOMATION_BAR_HEIGHT).toBe(76);
    expect(rowIndexAt(r, TRACK_HEIGHT + 10)).toBe(0);
  });

  it("computes clip rects in content px for the marquee", () => {
    const rects = clipRects(rows, [clip("x", "m2", 2, 4)], { pxPerBeat: 10, scrollBeats: 1 });
    expect(rects).toEqual([{ id: "x", rect: { x0: HEADER_WIDTH + 10, x1: HEADER_WIDTH + 50, y0: TRACK_HEIGHT, y1: 2 * TRACK_HEIGHT } }]);
  });

  it("summarizes clips inside a group", () => {
    const byId = Object.fromEntries(all.map((t) => [t.id, t]));
    const clips = { a: clip("a", "c1", 4, 2), b: audio("b", "c2", 0, 1), c: clip("c", "t", 0, 8) };
    expect(groupSummaryKey(byId, clips, "g")).toBe("0,1;4,2");
    expect(groupSummaryKey(byId, clips, "g2")).toBe("0,1");
  });

  it("finds the bar around a position", () => {
    expect(barAround(tempo, 5.5)).toEqual({ start: 4, length: 4 });
  });
});

// ── edit math ─────────────────────────────────────────────────────────────────────────

describe("dragPreview", () => {
  const a = clip("a", "m1", 4, 4);
  const b = clip("b", "m1", 10, 2);
  const input: DragInput = { clips: [a, b], anchor: "a", rows, snap: beatGrid };

  it("moves the selection by the anchor's snapped delta", () => {
    const p = dragPreview(input, "move", 1.4);
    expect(p.get("a")).toMatchObject({ start: 5, track: "m1" });
    expect(p.get("b")).toMatchObject({ start: 11, track: "m1" });
  });

  it("never moves the selection before 0", () => {
    const p = dragPreview(input, "move", -10);
    expect(p.get("a")!.start).toBe(0);
    expect(p.get("b")!.start).toBe(6);
  });

  it("moves between compatible tracks only", () => {
    expect(dragPreview(input, "move", 0, 1).get("a")!.track).toBe("m2");
    // Row +2 is an audio track: MIDI clips stay on their track.
    expect(dragPreview(input, "move", 0, 2).get("a")!.track).toBe("m1");
    expect(dragPreview(input, "move", 0, 9).get("a")!.track).toBe("m1");
  });

  it("resizes the end with snapping and a minimum length", () => {
    expect(dragPreview(input, "resize-end", 1.6).get("a")).toMatchObject({ start: 4, length: 6 });
    expect(dragPreview(input, "resize-end", -10).get("b")!.length).toBeGreaterThan(0);
  });

  it("limits unlooped audio resizes to the media length", () => {
    const x = audio("x", "a1", 0, 4);
    const p = dragPreview({ clips: [x], anchor: "x", rows, snap: beatGrid, sourceLength: () => 6 }, "resize-end", 10);
    expect(p.get("x")!.length).toBe(6);
    const looped = { ...x, looping: { enabled: true, start: 0, end: 4 } };
    const q = dragPreview({ clips: [looped], anchor: "x", rows, snap: beatGrid, sourceLength: () => 6 }, "resize-end", 10);
    expect(q.get("x")!.length).toBe(14);
  });

  it("resizes the start keeping the end and shifting the offset", () => {
    expect(dragPreview(input, "resize-start", 1.2).get("a")).toEqual({ track: "m1", start: 5, length: 3, offset: 1 });
    // Can't extend before the content start (offset 0).
    expect(dragPreview(input, "resize-start", -2).get("a")).toEqual({ track: "m1", start: 4, length: 4, offset: 0 });
  });
});

describe("commands", () => {
  const a = clip("a", "m1", 4, 4);
  const b = clip("b", "m1", 10, 2);
  let n = 0;
  const ids = () => `new${++n}`;

  it("moves only changed clips in one command", () => {
    const p = dragPreview({ clips: [a, b], anchor: "a", rows, snap: beatGrid }, "move", 2, 1);
    expect(moveCommand([a, b], p, false, ids)).toEqual({
      domain: "Clip",
      command: {
        type: "Move",
        moves: [
          { id: "a", track: "m2", start: 6 },
          { id: "b", track: "m2", start: 12 },
        ],
      },
    });
    const none = dragPreview({ clips: [a], anchor: "a", rows, snap: beatGrid }, "move", 0.2);
    expect(moveCommand([a], none, false, ids)).toBeNull();
  });

  it("copies as one batch (duplicate, then move to the new track)", () => {
    n = 0;
    const p = dragPreview({ clips: [a], anchor: "a", rows, snap: beatGrid }, "move", 4, 1);
    const c = moveCommand([a], p, true, ids)!;
    expect(c.domain).toBe("Edit");
    expect(commandsOf(c).map((x) => x.command.type)).toEqual(["Duplicate", "Move"]);
    expect(commandsOf(c)[0]!.command).toMatchObject({ id: "a", new_id: "new1", start: 8 });
  });

  it("resizes with SetBounds", () => {
    const p = dragPreview({ clips: [a], anchor: "a", rows, snap: beatGrid }, "resize-start", 1);
    expect(boundsCommand([a], p)).toEqual({
      domain: "Clip",
      command: { type: "SetBounds", id: "a", start: 5, length: 3, offset: 1 },
    });
  });

  it("splits only clips containing the position", () => {
    const c = commandsOf(splitCommand([a, b], 6, ids));
    expect(c).toHaveLength(1);
    expect(c[0]!.command).toMatchObject({ type: "Split", id: "a", at: 6 });
    expect(splitCommand([a], 4, ids)).toBeNull();
  });

  it("duplicates the selection as a block after itself", () => {
    const c = commandsOf(duplicateCommand([a, b], ids));
    expect(c.map((x) => (x.command as { start: number }).start)).toEqual([12, 18]);
  });

  it("toggles looping", () => {
    expect(toggleLoopCommand([a])!.command).toEqual({
      type: "SetLoop",
      id: "a",
      looping: { enabled: true, start: 0, end: 4 },
    });
    const looped = { ...a, looping: { enabled: true, start: 0, end: 2 } };
    expect(toggleLoopCommand([looped])!.command).toMatchObject({ looping: { enabled: false, start: 0, end: 2 } });
    // Mixed selection: loop the unlooped ones, leave the looped ones alone.
    expect(commandsOf(toggleLoopCommand([looped, b]))).toHaveLength(1);
  });
});

describe("laneItems", () => {
  const a = clip("a", "m1", 0, 4);
  const b = clip("b", "m2", 0, 4);
  const clips = { a, b };

  it("draws dragged clips in their target lane", () => {
    const bounds = new Map([["a", { track: "m2", start: 8, length: 4, offset: 0 }]]);
    expect(laneItems(clips, "m1", { bounds, copy: false })).toEqual([]);
    expect(laneItems(clips, "m2", { bounds, copy: false }).map((x) => [x.clip.id, x.bounds.start, x.dragging])).toEqual([
      ["b", 0, false],
      ["a", 8, true],
    ]);
  });

  it("keeps originals and adds ghosts for copies", () => {
    const bounds = new Map([["a", { track: "m1", start: 8, length: 4, offset: 0 }]]);
    expect(laneItems(clips, "m1", { bounds, copy: true }).map((x) => [x.clip.id, x.ghost])).toEqual([
      ["a", false],
      ["a", true],
    ]);
  });
});

// ── drawing helpers ───────────────────────────────────────────────────────────────────

describe("note preview", () => {
  const note = (start: number, duration: number, pitch = 60): Note => ({
    id: `n${start}`,
    clip: "c",
    pitch,
    velocity: 1,
    release_velocity: 0.5,
    start,
    duration,
    muted: false,
  });

  it("places notes on the timeline and repeats them in loops", () => {
    const c = clip("c", "m1", 8, 8, { looping: { enabled: true, start: 0, end: 4 } });
    const rects = noteRects(c, 8, [note(0, 1), note(3, 2)], 8, 16);
    expect(rects.map((r) => [r.t0, r.t1])).toEqual([
      [8, 9],
      [11, 12],
      [12, 13],
      [15, 16],
    ]);
  });

  it("pads the pitch range to an octave", () => {
    expect(pitchRange([note(0, 1, 60)])).toEqual({ lo: 54, hi: 66 });
    expect(pitchRange([])).toEqual({ lo: 60, hi: 72 });
  });
});

describe("peaks", () => {
  it("picks power-of-two levels", () => {
    expect(peakLevel(1)).toBe(16);
    expect(peakLevel(100)).toBe(128);
    expect(peakLevel(128)).toBe(128);
  });

  const tileData = (index: number, level: number, value: number): PeakData => ({
    media: "m",
    samples_per_peak: level,
    start_frame: index * TILE_PEAKS * level,
    min: [new Array(TILE_PEAKS).fill(-value)],
    max: [new Array(TILE_PEAKS).fill(value)],
  });

  it("merges peaks across tiles and reports missing ones", () => {
    const level = 16;
    const edge = TILE_PEAKS * level;
    const tiles = [tileData(0, level, 0.2), tileData(1, level, 0.5)];
    expect(peakRange(edge - 32, edge + 32, level, (i) => tiles[i] ?? null)).toEqual({ min: -0.5, max: 0.5 });
    expect(peakRange(0, 32, level, (i) => tiles[i] ?? null)).toEqual({ min: -0.2, max: 0.2 });
    expect(peakRange(0, 32, level, () => null)).toBeNull();
  });

  it("fetches each tile once and notifies when it arrives", async () => {
    const send = vi.fn(
      async (c: Command): Promise<ReplyValue> => {
        const r = (c.command as { request: { start_frame: number; samples_per_peak: number } }).request;
        return { type: "Peaks", peaks: tileData(r.start_frame / (TILE_PEAKS * r.samples_per_peak), r.samples_per_peak, 1) };
      },
    );
    const cache = new PeakCache({ send } as unknown as EngineTransport);
    const listener = vi.fn();
    cache.subscribe(listener);
    expect(cache.tile("m", 16, 2)).toBeNull();
    expect(cache.tile("m", 16, 2)).toBeNull();
    await Promise.resolve();
    await Promise.resolve();
    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0]![0]).toEqual({
      domain: "Media",
      command: { type: "GetPeaks", request: { media: "m", samples_per_peak: 16, start_frame: 2 * TILE_PEAKS * 16, frame_count: TILE_PEAKS * 16 } },
    });
    expect(listener).toHaveBeenCalled();
    expect(cache.tile("m", 16, 2)?.start_frame).toBe(2 * TILE_PEAKS * 16);
    cache.invalidate("m");
    expect(cache.tile("m", 16, 2)).toBeNull();
    expect(send).toHaveBeenCalledTimes(2);
  });
});

// ── misc ──────────────────────────────────────────────────────────────────────────────

describe("follow playhead", () => {
  const s = { pxPerBeat: 10, scrollBeats: 0, widthPx: 1000 };
  it("pages before the playhead reaches the edge, keeping a margin", () => {
    expect(followScroll(s, 50)).toBeNull();
    const to = followScroll(s, 95)!;
    expect(to).toBeCloseTo(95 - 8);
    expect(followScroll({ ...s, scrollBeats: 200 }, 10)).toBeCloseTo(2);
  });
});

describe("keys and drops", () => {
  const k = (key: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) =>
    actionForKey({ key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });

  it("maps shortcuts", () => {
    expect(k("Delete")).toBe("delete");
    expect(k("Backspace")).toBe("delete");
    expect(k("e", { ctrlKey: true })).toBe("split");
    expect(k("d", { metaKey: true })).toBe("duplicate");
    expect(k("L", { metaKey: true, shiftKey: true })).toBe("loop");
    expect(k("a", { ctrlKey: true })).toBe("select-all");
    expect(k("e")).toBeNull();
  });

  it("reads the browser drag payload", () => {
    const payload = { version: 1, kind: "media", source: { type: "Project", media: "m" }, name: "x.wav", file_kind: "Audio" };
    const dt = (data: string) => ({ types: [BROWSER_DRAG_MIME], getData: () => data });
    expect(readBrowserDrag(dt(JSON.stringify(payload)))).toEqual(payload);
    expect(readBrowserDrag(dt("{"))).toBeNull();
    expect(readBrowserDrag(dt(JSON.stringify({ ...payload, version: 2 })))).toBeNull();
    expect(readBrowserDrag({ types: ["text/plain"], getData: () => "" })).toBeNull();
  });
});
