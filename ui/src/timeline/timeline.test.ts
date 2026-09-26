import { describe, expect, it } from "vitest";
import {
  applyLoopDrag,
  beatsToPx,
  createTimelineViewStore,
  formatBarBeat,
  formatDuration,
  formatSeconds,
  loopFromPoints,
  pxToBeats,
  pxToSeconds,
  revealBeats,
  rulerMarks,
  secondsToPx,
  TempoMap,
  visibleRange,
  zoomBy,
  zoomToRange,
  type TimelineViewport,
} from ".";

describe("timeline mapping", () => {
  const vp: TimelineViewport = { pxPerBeat: 20, scrollBeats: 4 };

  it("maps beats to pixels relative to the scroll position", () => {
    expect(beatsToPx(4, vp)).toBe(0);
    expect(beatsToPx(8, vp)).toBe(80);
    expect(beatsToPx(2, vp)).toBe(-40);
  });

  it("pxToBeats inverts beatsToPx", () => {
    for (const b of [0, 1.5, 4, 17.25]) expect(pxToBeats(beatsToPx(b, vp), vp)).toBeCloseTo(b);
  });

  it("maps seconds through the tempo map", () => {
    const tempo = TempoMap.constant(120);
    expect(secondsToPx(2, vp, tempo)).toBeCloseTo(0); // 2 s = beat 4
    expect(pxToSeconds(80, vp, tempo)).toBeCloseTo(4);
  });

  it("visible range", () => {
    expect(visibleRange(vp, 200)).toEqual({ start: 4, end: 14 });
  });
});

describe("zoom", () => {
  const vp: TimelineViewport = { pxPerBeat: 20, scrollBeats: 4 };

  it("zooms around the anchor", () => {
    const z = zoomBy(vp, 2, 100);
    expect(z.pxPerBeat).toBe(40);
    expect(pxToBeats(100, z)).toBeCloseTo(pxToBeats(100, vp));
  });

  it("clamps zoom and scroll", () => {
    const z = zoomBy(vp, 1e9, 0);
    expect(z.pxPerBeat).toBe(2000);
    const out = zoomBy({ pxPerBeat: 20, scrollBeats: 0 }, 0.5, 100);
    expect(out.scrollBeats).toBe(0);
  });

  it("fits a range", () => {
    const z = zoomToRange({ start: 8, end: 16 }, 400, 0);
    expect(z).toEqual({ pxPerBeat: 50, scrollBeats: 8 });
  });

  it("reveals a position only when needed", () => {
    expect(revealBeats(vp, 6, 200)).toBe(vp);
    expect(revealBeats(vp, 20, 200).scrollBeats).toBeCloseTo(10);
    expect(revealBeats(vp, 2, 200).scrollBeats).toBeCloseTo(2);
  });
});

describe("view store", () => {
  it("zooms around the center and scrolls by pixels", () => {
    const s = createTimelineViewStore({ pxPerBeat: 10, widthPx: 400 });
    s.getState().zoomBy(2);
    expect(s.getState().pxPerBeat).toBe(20);
    expect(s.getState().scrollBeats).toBeCloseTo(10); // center beat 20 stays at 200px
    s.getState().scrollByPx(40);
    expect(s.getState().scrollBeats).toBeCloseTo(12);
    s.getState().scrollTo(-5);
    expect(s.getState().scrollBeats).toBe(0);
    expect(s.getState().visibleRange()).toEqual({ start: 0, end: 20 });
  });

  it("does not notify when nothing changes", () => {
    const s = createTimelineViewStore();
    let n = 0;
    s.subscribe(() => n++);
    s.getState().scrollTo(0);
    s.getState().setWidth(0);
    expect(n).toBe(0);
  });
});

describe("format", () => {
  const tempo = TempoMap.constant(120);
  it("bar.beat.sixteenth", () => {
    expect(formatBarBeat(0, tempo)).toBe("1.1.1");
    expect(formatBarBeat(5.25, tempo)).toBe("2.2.2");
    expect(formatBarBeat(5.25, tempo, 2)).toBe("2.2");
    expect(formatBarBeat(5.25, tempo, 1)).toBe("2");
  });
  it("durations and seconds", () => {
    expect(formatDuration(4)).toBe("1.0.0");
    expect(formatDuration(5.75)).toBe("1.1.3");
    expect(formatSeconds(65.5)).toBe("1:05.500");
    expect(formatSeconds(-1, false)).toBe("-0:01");
  });
});

describe("ruler marks", () => {
  const tempo = TempoMap.constant(120);
  it("labels bars at a readable spacing", () => {
    const m = rulerMarks(tempo, { pxPerBeat: 5, scrollBeats: 0 }, 400, "bars");
    // 20 px per bar → label every 4 bars (80 px ≥ 44).
    expect(m.labels.map((l) => l.text)).toEqual(["1", "5", "9", "13", "17"]);
    expect(m.ticks.every((t) => t.x >= 0 && t.x < 400)).toBe(true);
    expect(m.ticks.filter((t) => t.major).length).toBeGreaterThan(0);
  });
  it("labels beats when zoomed in", () => {
    const m = rulerMarks(tempo, { pxPerBeat: 60, scrollBeats: 0 }, 250, "bars");
    expect(m.labels.map((l) => l.text)).toEqual(["1", "1.2", "1.3", "1.4", "2"]);
  });
  it("seconds ruler", () => {
    const m = rulerMarks(tempo, { pxPerBeat: 20, scrollBeats: 0 }, 400, "seconds");
    // 40 px per second → labels every 2 s.
    expect(m.labels.map((l) => l.text)).toEqual(["0:00", "0:02", "0:04", "0:06", "0:08", "0:10"]);
  });
});

describe("loop editing", () => {
  const r = { start: 4, end: 8 };
  const snap = (b: number) => Math.round(b);
  it("moves, resizes and keeps a minimum length", () => {
    expect(applyLoopDrag(r, "move", 1.3, snap)).toEqual({ start: 5, end: 9 });
    expect(applyLoopDrag(r, "move", -10, snap)).toEqual({ start: 0, end: 4 });
    expect(applyLoopDrag(r, "start", 1.6, snap)).toEqual({ start: 6, end: 8 });
    expect(applyLoopDrag(r, "end", -10, snap).end).toBeCloseTo(4 + 1 / 16);
    expect(loopFromPoints(6, 2)).toEqual({ start: 2, end: 6 });
  });
});
