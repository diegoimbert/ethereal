import { afterEach, describe, expect, it } from "vitest";
import type { Track, TrackId } from "@/generated";
import { duration, ease, motion } from "@/theme";
import { MOTION } from "@/timeline/motion";
import { layoutRows, rowIndexAt, TRACK_HEIGHT } from "@/features/arrangement/layout";
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
