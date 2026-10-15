import { describe, expect, it, vi } from "vitest";
import { DOUBLE_TAP_MS, FLING_TIME_CONSTANT_MS, LONG_PRESS_MS, SLOP_PX } from "./constants";
import { flingDistance, flingStep, shouldFling, startFling, VelocityTracker } from "./momentum";
import { GestureRecognizer, type Gesture } from "./recognizer";
import { createTimelineViewStore } from "@/timeline";
import { keepCenterOnRotation } from "../rotation";

function setup() {
  const out: Gesture[] = [];
  const r = new GestureRecognizer((g) => out.push(g));
  const types = () => out.map((g) => g.type);
  return { r, out, types };
}

describe("GestureRecognizer: one finger", () => {
  it("a press released in place is a tap", () => {
    const { r, out } = setup();
    r.down(1, { x: 10, y: 10 }, 0);
    r.move(1, { x: 13, y: 12 }, 50);
    r.up(1, { x: 13, y: 12 }, 100);
    expect(out).toEqual([{ type: "tap", x: 10, y: 10, count: 1 }]);
  });

  it("a second tap soon after and near the first is a double-tap; a third starts over", () => {
    const { r, out } = setup();
    const tap = (t: number, x = 10) => {
      r.down(1, { x, y: 10 }, t);
      r.up(1, { x, y: 10 }, t + 40);
    };
    tap(0);
    tap(200, 20);
    tap(400);
    expect(out.map((g) => (g.type === "tap" ? g.count : 0))).toEqual([1, 2, 1]);
  });

  it("a press elsewhere between two taps breaks the double-tap", () => {
    const { r, out } = setup();
    r.down(1, { x: 10, y: 10 }, 0);
    r.up(1, { x: 10, y: 10 }, 40);
    r.forgetTap();
    r.down(1, { x: 10, y: 10 }, 100);
    r.up(1, { x: 10, y: 10 }, 140);
    expect(out.map((g) => (g.type === "tap" ? g.count : 0))).toEqual([1, 1]);
  });

  it("taps too far apart in time or space are two single taps", () => {
    const { r, out } = setup();
    r.down(1, { x: 10, y: 10 }, 0);
    r.up(1, { x: 10, y: 10 }, 40);
    r.down(1, { x: 10, y: 10 }, 40 + DOUBLE_TAP_MS + 1);
    r.up(1, { x: 10, y: 10 }, 40 + DOUBLE_TAP_MS + 2);
    r.down(1, { x: 60, y: 10 }, 1000);
    r.up(1, { x: 60, y: 10 }, 1001);
    r.down(1, { x: 10, y: 10 }, 1100);
    r.up(1, { x: 10, y: 10 }, 1101);
    expect(out.map((g) => (g.type === "tap" ? g.count : 0))).toEqual([1, 1, 1, 1]);
  });

  it("held still: long-press, then long-press-up on release (no tap)", () => {
    const { r, types } = setup();
    r.down(1, { x: 10, y: 10 }, 0);
    expect(r.waitingForLongPress).toBe(true);
    r.timer(LONG_PRESS_MS - 1);
    expect(types()).toEqual([]);
    r.timer(LONG_PRESS_MS);
    expect(r.waitingForLongPress).toBe(false);
    r.up(1, { x: 11, y: 10 }, 900);
    expect(types()).toEqual(["longpress", "longpress-up"]);
  });

  it("moving past the slop pans, locked to the axis it starts along, applying the whole movement", () => {
    const { r, out } = setup();
    r.down(1, { x: 100, y: 100 }, 0);
    r.move(1, { x: 100 + SLOP_PX - 1, y: 100 }, 10);
    expect(out).toEqual([]);
    r.move(1, { x: 112, y: 102 }, 20);
    expect(out[0]).toMatchObject({ type: "pan-start", axis: "x", after: "press", startX: 100, startY: 100 });
    expect(out[1]).toEqual({ type: "pan", x: 112, y: 102, dx: 12, dy: 0 });
    // Locked: vertical drift is ignored.
    r.move(1, { x: 120, y: 130 }, 30);
    expect(out[2]).toEqual({ type: "pan", x: 120, y: 130, dx: 8, dy: 0 });
    r.up(1, { x: 120, y: 130 }, 40);
    expect(out[3]).toMatchObject({ type: "pan-end", vy: 0 });
  });

  it("a vertical start locks to y; a diagonal start pans freely", () => {
    const a = setup();
    a.r.down(1, { x: 0, y: 0 }, 0);
    a.r.move(1, { x: 2, y: 20 }, 10);
    expect(a.out[0]).toMatchObject({ type: "pan-start", axis: "y" });
    expect(a.out[1]).toMatchObject({ dx: 0, dy: 20 });
    const b = setup();
    b.r.down(1, { x: 0, y: 0 }, 0);
    b.r.move(1, { x: 10, y: 10 }, 10);
    expect(b.out[0]).toMatchObject({ type: "pan-start", axis: "free" });
    expect(b.out[1]).toMatchObject({ dx: 10, dy: 10 });
  });

  it("moving after a long-press is a free drag that started from the long-press (no menu)", () => {
    const { r, out, types } = setup();
    r.down(1, { x: 10, y: 10 }, 0);
    r.timer(LONG_PRESS_MS);
    r.move(1, { x: 40, y: 12 }, 600);
    r.up(1, { x: 40, y: 12 }, 700);
    expect(types()).toEqual(["longpress", "pan-start", "pan", "pan-end"]);
    expect(out[1]).toMatchObject({ after: "longpress", axis: "free" });
  });

  it("a pan's release speed comes from the last moments; a rest before lifting kills it", () => {
    const fast = setup();
    fast.r.down(1, { x: 0, y: 0 }, 0);
    for (let i = 1; i <= 10; i++) fast.r.move(1, { x: i * 10, y: 0 }, i * 10);
    fast.r.up(1, { x: 100, y: 0 }, 100);
    const end = fast.out.at(-1) as Extract<Gesture, { type: "pan-end" }>;
    expect(end.vx).toBeCloseTo(1, 1);
    const rested = setup();
    rested.r.down(1, { x: 0, y: 0 }, 0);
    for (let i = 1; i <= 10; i++) rested.r.move(1, { x: i * 10, y: 0 }, i * 10);
    rested.r.up(1, { x: 100, y: 0 }, 400);
    expect((rested.out.at(-1) as Extract<Gesture, { type: "pan-end" }>).vx).toBe(0);
  });
});

describe("GestureRecognizer: two fingers", () => {
  it("fingers side by side pinch time: factor from the horizontal spread, midpoint motion", () => {
    const { r, out } = setup();
    r.down(1, { x: 100, y: 200 }, 0);
    r.down(2, { x: 200, y: 210 }, 10);
    expect(out[0]).toEqual({ type: "pinch-start", x: 150, y: 205, axis: "x" });
    r.move(2, { x: 300, y: 210 }, 20);
    expect(out[1]).toMatchObject({ type: "pinch", axis: "x", scale: 2, dx: 50, dy: 0 });
    r.up(1, { x: 100, y: 200 }, 30);
    expect(out[2]).toEqual({ type: "pinch-end" });
    // The finger left behind does nothing until it lifts.
    r.move(2, { x: 400, y: 300 }, 40);
    r.up(2, { x: 400, y: 300 }, 50);
    expect(out).toHaveLength(3);
    expect(r.active).toBe(false);
  });

  it("fingers one above the other pinch height", () => {
    const { r, out } = setup();
    r.down(1, { x: 100, y: 100 }, 0);
    r.down(2, { x: 110, y: 300 }, 10);
    expect(out[0]).toMatchObject({ type: "pinch-start", axis: "y" });
    r.move(2, { x: 110, y: 200 }, 20);
    expect(out[1]).toMatchObject({ type: "pinch", axis: "y", scale: 0.5 });
  });

  it("a second finger cancels a pan (a drag drops its preview) and becomes a pinch", () => {
    const { r, types } = setup();
    r.down(1, { x: 0, y: 0 }, 0);
    r.move(1, { x: 30, y: 0 }, 10);
    r.down(2, { x: 100, y: 0 }, 20);
    expect(types()).toEqual(["pan-start", "pan", "cancel", "pinch-start"]);
  });

  it("nearly touching fingers don't give huge factors", () => {
    const { r, out } = setup();
    r.down(1, { x: 100, y: 0 }, 0);
    r.down(2, { x: 102, y: 0 }, 10);
    r.move(2, { x: 150, y: 0 }, 20);
    const pinch = out[1] as Extract<Gesture, { type: "pinch" }>;
    expect(pinch.scale).toBeLessThan(2.1);
  });

  it("a pointercancel ends the gesture", () => {
    const { r, types } = setup();
    r.down(1, { x: 0, y: 0 }, 0);
    r.move(1, { x: 30, y: 0 }, 10);
    r.cancel();
    expect(types()).toEqual(["pan-start", "pan", "cancel"]);
    expect(r.active).toBe(false);
  });
});

describe("keepCenterOnRotation", () => {
  it("a window width change keeps the centre beat; a lanes-only change keeps the left edge", () => {
    const view = createTimelineViewStore({ pxPerBeat: 10 });
    view.getState().setWidth(400);
    view.getState().scrollTo(100);
    let windowWidth = 540;
    const off = keepCenterOnRotation(view, () => windowWidth);
    const center = () => view.getState().scrollBeats + view.getState().widthPx / 2 / view.getState().pxPerBeat;
    expect(center()).toBe(120);
    // Rotated to landscape: the lanes widen, the centre stays.
    windowWidth = 900;
    view.getState().setWidth(760);
    expect(center()).toBeCloseTo(120, 6);
    // The header column resized (same window): the left edge stays.
    const left = view.getState().scrollBeats;
    view.getState().setWidth(700);
    expect(view.getState().scrollBeats).toBe(left);
    off();
  });
});

describe("momentum", () => {
  it("a fling decays exponentially and covers v·τ in total", () => {
    let v = 2;
    let total = 0;
    for (let i = 0; i < 1000 && v !== 0; i++) {
      const s = flingStep(v, 16);
      total += s.delta;
      v = s.v;
    }
    expect(v).toBe(0);
    expect(total).toBeGreaterThan(flingDistance(2) * 0.95);
    expect(total).toBeLessThanOrEqual(flingDistance(2));
    expect(flingDistance(1)).toBe(FLING_TIME_CONSTANT_MS);
  });

  it("slow releases don't fling", () => {
    expect(shouldFling(0.05, 0.05)).toBe(false);
    expect(shouldFling(0.3, 0)).toBe(true);
    const apply = vi.fn();
    startFling(0.01, 0, apply, () => 0);
    expect(apply).not.toHaveBeenCalled();
  });

  it("startFling runs on frames until it stops, and can be stopped", () => {
    const frames: Array<(t: number) => void> = [];
    const apply = vi.fn();
    const stop = startFling(
      1,
      0,
      apply,
      (cb) => frames.push(cb),
      () => frames.splice(0),
    );
    frames.shift()!(0);
    frames.shift()!(16);
    expect(apply).toHaveBeenCalledTimes(2);
    expect(apply.mock.calls[0]![0]).toBeGreaterThan(apply.mock.calls[1]![0]);
    stop();
    expect(frames).toHaveLength(0);
  });

  it("VelocityTracker: px per ms over the window", () => {
    const v = new VelocityTracker();
    v.reset({ x: 0, y: 0 }, 0);
    v.add({ x: 50, y: -25 }, 50);
    expect(v.speed(50)).toEqual({ x: 1, y: -0.5 });
    expect(v.speed(500)).toEqual({ x: 0, y: 0 });
  });
});
