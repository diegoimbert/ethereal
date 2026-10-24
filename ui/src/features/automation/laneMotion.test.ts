import { afterEach, describe, expect, it } from "vitest";
import type { Track, TrackId } from "@/generated";
import { duration, ease, motion } from "@/theme";
import { MOTION } from "@/timeline/motion";
import { layoutRows, rowIndexAt, TRACK_HEIGHT } from "@/features/arrangement/layout";
import { animatedRows, collapseLeaving } from "@/features/arrangement/laneAnimation";
import {
  changedHeights,
  cubicBezier,
  HeightAnimator,
  laneAnimator,
  mergeKeys,
  nextShownLanes,
  parseDuration,
  resetLaneMotion,
  restingLanes,
  useLaneMotion,
} from "./laneMotion";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, resetAutomationUi, useAutomationUi } from "./uiStore";

const D = 300;
const make = (enabled = true) => new HeightAnimator({ duration: D, ease: cubicBezier(ease.standard), enabled: () => enabled });
const OPEN = AUTOMATION_BAR_HEIGHT + LANE_HEIGHT;

afterEach(() => {
  MOTION.enabled = false;
  resetAutomationUi();
  resetLaneMotion();
});

describe("tokens", () => {
  it("lane motion uses the design tokens", () => {
    expect(motion.lane.duration).toBe(duration.slow);
    expect(parseDuration(motion.lane.duration)).toBe(300);
    expect(parseDuration("0.15s")).toBe(150);
    expect(motion.lane.ease).toBe(ease.standard);
  });

  it("cubicBezier matches its end points and is monotonic", () => {
    const f = cubicBezier(ease.standard);
    expect(f(0)).toBe(0);
    expect(f(1)).toBe(1);
    let last = 0;
    for (let p = 0.01; p <= 1; p += 0.01) {
      const y = f(p);
      expect(y).toBeGreaterThanOrEqual(last - 1e-9);
      last = y;
    }
    // ease-standard (0.2, 0, 0, 1) is decelerating: well past halfway at half time.
    expect(f(0.5)).toBeGreaterThan(0.7);
    expect(cubicBezier("linear")(0.25)).toBe(0.25);
  });
});

describe("HeightAnimator", () => {
  it("opens monotonically and reaches the target", () => {
    const a = make();
    a.retarget("T", 0, OPEN, 1000);
    expect(a.height("T", OPEN)).toBe(0);
    let last = 0;
    for (let t = 1000; t <= 1000 + D; t += 16) {
      a.tick(t);
      const h = a.height("T", OPEN);
      expect(h).toBeGreaterThanOrEqual(last);
      expect(h).toBeLessThanOrEqual(OPEN);
      last = h;
    }
    expect(a.tick(1000 + D)).toEqual(["T"]);
    expect(a.tick(1000 + D + 1)).toEqual([]);
    expect(a.height("T", OPEN)).toBe(OPEN);
    expect(a.active).toBe(false);
  });

  it("ends exactly on the target and reports the end", () => {
    const a = make();
    a.retarget("T", OPEN, 0, 0);
    a.tick(100);
    expect(a.isAnimating("T")).toBe(true);
    expect(a.tick(D)).toEqual(["T"]);
    expect(a.height("T", 0)).toBe(0);
  });

  it("reverses mid-animation from what is on screen, without a jump", () => {
    const a = make();
    a.retarget("T", 0, OPEN, 0);
    a.tick(D / 3);
    const mid = a.height("T", OPEN);
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(OPEN);
    a.retarget("T", OPEN, 0, D / 3);
    expect(a.height("T", 0)).toBe(mid);
    let last = mid;
    for (let t = D / 3; t <= D / 3 + D; t += 16) {
      a.tick(t);
      const h = a.height("T", 0);
      expect(h).toBeLessThanOrEqual(last);
      last = h;
    }
    a.tick(D / 3 + D);
    expect(a.height("T", 0)).toBe(0);
  });

  it("is instant with reduced motion", () => {
    const a = make(false);
    a.retarget("T", 0, OPEN, 0);
    expect(a.isAnimating("T")).toBe(false);
    expect(a.height("T", OPEN)).toBe(OPEN);
  });

  it("layout hit testing uses the animated height", () => {
    const tracks = [
      { id: "A", kind: "Audio", name: "A", color: 0, order: "a", parent: null, mixer: { volume: 0, pan: 0, mute: false, solo: false } },
      { id: "B", kind: "Audio", name: "B", color: 0, order: "b", parent: null, mixer: { volume: 0, pan: 0, mute: false, solo: false } },
    ] as unknown as Track[];
    const a = make();
    a.retarget("A", 0, OPEN, 0);
    a.tick(D / 2);
    const h = a.height("A", OPEN);
    const rows = layoutRows(tracks, new Set(), (id: TrackId) => (id === "A" ? a.height(id, OPEN) : 0));
    expect(rows[0]!.height).toBe(TRACK_HEIGHT + h);
    expect(rows[1]!.y).toBe(TRACK_HEIGHT + h);
    // Just above B's (animated) top is still A; at it, B.
    expect(rowIndexAt(rows, TRACK_HEIGHT + h - 0.5)).toBe(0);
    expect(rowIndexAt(rows, TRACK_HEIGHT + h)).toBe(1);
    // Where B will be once open is still A's automation mid-animation... and vice versa.
    expect(rowIndexAt(rows, TRACK_HEIGHT + OPEN - 1)).toBe(1);

    // The arrangement lays out once at the larger height, then derives the on-screen rows
    // each frame (`animatedRows`): same model as laying out with the animated height.
    const laid = layoutRows(tracks, new Set(), (id: TrackId) => (id === "A" ? OPEN : 0));
    const shown = animatedRows(laid, (r) => (r.track.id === "A" ? a.height("A", OPEN) : 0));
    expect(shown.map((r) => [r.y, r.height])).toEqual(rows.map((r) => [r.y, r.height]));
    expect(animatedRows(laid, (r) => r.height - r.laneHeight)[1]).toBe(laid[1]);
  });

  it("a group folding mid-animation: its rows at a share of their height (lanes too), the rows below moved up", () => {
    const mixer = { volume: 0, pan: 0, mute: false, solo: false };
    const tracks = [
      { id: "G", kind: "Group", name: "G", color: 0, order: "a", parent: null, mixer },
      { id: "C1", kind: "Audio", name: "C1", color: 0, order: "a", parent: "G", mixer },
      { id: "C2", kind: "Audio", name: "C2", color: 0, order: "b", parent: "G", mixer },
      { id: "B", kind: "Audio", name: "B", color: 0, order: "b", parent: null, mixer },
    ] as unknown as Track[];
    const laid = layoutRows(tracks, new Set(), () => 0);
    const shown = animatedRows(
      laid,
      () => 0,
      (r) => (r.track.parent === "G" ? 0.25 : 1),
    );
    expect(shown.map((r) => r.track.id)).toEqual(["G", "C1", "C2", "B"]);
    expect(shown[1]).toMatchObject({ y: TRACK_HEIGHT, height: TRACK_HEIGHT / 4, laneHeight: TRACK_HEIGHT / 4 });
    expect(shown[2]!.y).toBe(TRACK_HEIGHT * 1.25);
    expect(shown[3]!.y).toBe(TRACK_HEIGHT * 1.5);
    // A folding row carries its laid-out row: the arranger draws its clips at that size (not
    // compact on the way), cut at the row's height, without text.
    expect(shown[1]!.folding).toBe(laid[1]);
    expect(shown[3]!.folding).toBeUndefined();
    // Not folding: the rows as laid out.
    expect(animatedRows(laid, () => 0)[3]).toBe(laid[3]);
  });
});

describe("hiding a lane of an open track", () => {
  const tracks = [
    { id: "A", kind: "Audio", name: "A", color: 0, order: "a", parent: null, mixer: { volume: 0, pan: 0, mute: false, solo: false } },
    { id: "B", kind: "Audio", name: "B", color: 0, order: "b", parent: null, mixer: { volume: 0, pan: 0, mute: false, solo: false } },
  ] as unknown as Track[];

  // Lanes [a, b, c] (heights below), one hidden: the first, a middle, the last, the only one;
  // default and compact (minimum-height) lanes.
  const cases: Array<{ name: string; lanes: number[]; hide: number }> = [
    { name: "the first", lanes: [64, 64, 64], hide: 0 },
    { name: "a middle", lanes: [64, 96, 64], hide: 1 },
    { name: "the last", lanes: [64, 64, 64], hide: 2 },
    { name: "the only", lanes: [64], hide: 0 },
    { name: "a compact middle", lanes: [32, 32, 32], hide: 1 },
  ];

  for (const c of cases) {
    it(`${c.name} lane: what follows moves by exactly the layout change, every frame`, () => {
      const from = AUTOMATION_BAR_HEIGHT + c.lanes.reduce((x, y) => x + y, 0);
      const removed = c.lanes[c.hide]!;
      const to = from - removed;
      const a = make();
      a.retarget("A", from, to, 0);
      // Laid out once at the larger height (`useAutomationSlotHeight`).
      const laid = layoutRows(tracks, new Set(), (id) => (id === "A" ? Math.max(from, to) : 0));
      const after = layoutRows(tracks, new Set(), (id) => (id === "A" ? to : 0));
      const delta = after[1]!.y - laid[1]!.y;
      expect(delta).toBe(-removed);
      let last = 0;
      for (let t = 0; t <= D; t += D / 12) {
        a.tick(t);
        const slot = a.height("A", to);
        const shown = animatedRows(laid, (r) => (r.track.id === "A" ? slot : 0));
        const moved = shown[1]!.y - laid[1]!.y;
        // The rows below: between where they were and where they end (no overshoot), only up.
        expect(moved).toBeLessThanOrEqual(0);
        expect(moved).toBeGreaterThanOrEqual(delta);
        expect(moved).toBeLessThanOrEqual(last);
        last = moved;
        // The lanes under the hidden one move up by its collapse: exactly as much.
        const [h] = collapseLeaving(slot, to, [removed]);
        expect(h! - removed).toBeCloseTo(moved, 9);
        // The slot's content (bar, lanes kept, the hidden one collapsing) fills it exactly.
        expect(to + h!).toBeCloseTo(slot, 9);
      }
      expect(last).toBe(delta);
    });
  }

  it("several lanes leaving give up the difference in proportion", () => {
    expect(collapseLeaving(84 + 48, 84, [64, 32])).toEqual([32, 16]);
    expect(collapseLeaving(84, 84, [64, 32])).toEqual([0, 0]);
    // Clamped: never taller than they are, never negative.
    expect(collapseLeaving(500, 84, [64])).toEqual([64]);
    expect(collapseLeaving(20, 84, [64])).toEqual([0]);
    expect(collapseLeaving(100, 84, [])).toEqual([]);
  });
});

describe("app-wide lane motion", () => {
  it("tweens a track when the UI store opens it, and settles", () => {
    MOTION.enabled = true;
    useAutomationUi.getState().setOpen("T", true, ["k"]);
    expect(useLaneMotion.getState().animating.has("T")).toBe(true);
    expect(laneAnimator.height("T", OPEN)).toBe(0);
    laneAnimator.tick(performance.now() + 10_000);
    expect(laneAnimator.height("T", OPEN)).toBe(OPEN);
  });

  it("does nothing with reduced motion", () => {
    MOTION.enabled = false;
    useAutomationUi.getState().setOpen("T", true, ["k"]);
    expect(useLaneMotion.getState().animating.size).toBe(0);
  });

  it("changedHeights lists the tracks whose slot height changed", () => {
    const prev = { open: new Set<TrackId>(["A"]), shown: { A: ["x"], B: ["y"] } };
    const next = { open: new Set<TrackId>(["A", "B"]), shown: { A: ["x", "z"], B: ["y"] } };
    expect(changedHeights(prev, next)).toEqual([
      { track: "A", from: OPEN, to: OPEN + LANE_HEIGHT },
      { track: "B", from: 0, to: OPEN },
    ]);
  });
});

describe("shown lanes while animating", () => {
  it("opening: bar and lanes enter", () => {
    const s = nextShownLanes(restingLanes(false, []), true, ["a", "b"], true);
    expect(s.open).toBe(true);
    expect(s.barEntering).toBe(true);
    expect([...s.entering]).toEqual(["a", "b"]);
  });

  it("closing: keeps what was shown until the tween ends", () => {
    const s = nextShownLanes(restingLanes(true, ["a"]), false, [], true);
    expect(s).toMatchObject({ open: true, keys: ["a"], closing: true });
    expect(nextShownLanes(s, false, [], false)).toEqual(restingLanes(false, []));
  });

  it("adding a lane: only the new one enters", () => {
    const s = nextShownLanes(restingLanes(true, ["a"]), true, ["a", "b"], true);
    expect(s.barEntering).toBe(false);
    expect([...s.entering]).toEqual(["b"]);
  });

  it("hiding a lane: it stays in place and collapses", () => {
    const s = nextShownLanes(restingLanes(true, ["a", "b", "c"]), true, ["a", "c"], true);
    expect(s.keys).toEqual(["a", "b", "c"]);
    expect([...s.leaving]).toEqual(["b"]);
  });

  it("mergeKeys keeps removed items at their place", () => {
    expect(mergeKeys(["a", "b", "c"], ["c", "a"])).toEqual(["c", "a", "b"]);
    expect(mergeKeys(["a", "b"], ["b"])).toEqual(["a", "b"]);
  });
});
