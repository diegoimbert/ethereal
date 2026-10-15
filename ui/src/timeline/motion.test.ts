import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getMotionLevel, MOTION, ScaleFollower, ScrollYFollower, setMotionLevel, spring, stepSpring } from "./motion";
import { animatePan, animateZoom } from "./viewMotion";
import { pxToBeats } from "./viewport";
import { createTimelineViewStore } from "./viewStore";

beforeEach(() => {
  MOTION.enabled = true;
  vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance"] });
});

afterEach(() => {
  vi.useRealTimers();
  MOTION.enabled = false;
});

const frames = (n: number) => {
  for (let i = 0; i < n; i++) vi.advanceTimersByTime(16);
};

describe("stepSpring", () => {
  it("is framerate independent and halves the distance per half-life", () => {
    const a = { ...spring(0), goal: 1 };
    const b = { ...spring(0), goal: 1 };
    stepSpring(a, 0.1, 0.2);
    for (let i = 0; i < 20; i++) stepSpring(b, 0.1, 0.01);
    expect(a.x).toBeCloseTo(b.x, 9);
    expect(a.v).toBeCloseTo(b.v, 9);
    const c = { ...spring(0), goal: 1 };
    for (let i = 0; i < 100; i++) stepSpring(c, 0.1, 0.01);
    expect(c.x).toBeLessThanOrEqual(1); // critically damped: no overshoot from rest
    expect(c.x).toBeGreaterThan(0.99);
  });

  it("keeps momentum when the goal reverses", () => {
    const s = { ...spring(0), goal: 10 };
    for (let i = 0; i < 3; i++) stepSpring(s, 0.06, 0.016);
    const v = s.v;
    s.goal = -10;
    stepSpring(s, 0.06, 0.001);
    expect(s.v).toBeGreaterThan(0); // still moving the old way, decelerating
    expect(s.v).toBeLessThan(v);
  });
});

describe("animated view zoom/pan", () => {
  it("accumulates a burst of zooms, keeping the beat under the pointer fixed throughout", () => {
    const view = createTimelineViewStore({ pxPerBeat: 20, scrollBeats: 10, widthPx: 800 });
    const anchorBeat = pxToBeats(300, view.getState());
    animateZoom(view, 1.5, 300);
    animateZoom(view, 1.5, 300);
    animateZoom(view, 1.5, 300);
    expect(view.getState().pxPerBeat).toBe(20); // nothing moves before the first frame
    frames(3);
    const mid = view.getState();
    expect(mid.pxPerBeat).toBeGreaterThan(20);
    expect(mid.pxPerBeat).toBeLessThan(20 * 1.5 ** 3);
    expect(pxToBeats(300, mid)).toBeCloseTo(anchorBeat, 6);
    frames(200);
    expect(view.getState().pxPerBeat).toBeCloseTo(20 * 1.5 ** 3, 6);
    expect(pxToBeats(300, view.getState())).toBeCloseTo(anchorBeat, 6);
  });

  it("a quick zoom out with the pointer far right that runs into the song start eases the scroll straight there (no back and forth)", () => {
    const view = createTimelineViewStore({ pxPerBeat: 40, scrollBeats: 6, widthPx: 1000 });
    const scrolls: number[] = [];
    for (let i = 0; i < 40; i++) {
      if (i < 6) animateZoom(view, 1 / 1.2, 950);
      frames(1);
      scrolls.push(view.getState().scrollBeats);
    }
    for (let i = 1; i < scrolls.length; i++) expect(scrolls[i]).toBeLessThanOrEqual(scrolls[i - 1]! + 1e-9);
    expect(view.getState().scrollBeats).toBeCloseTo(0, 3);
    expect(view.getState().pxPerBeat).toBeCloseTo(40 / 1.2 ** 6, 3);
  });

  it("reverses smoothly and settles on the net result", () => {
    const view = createTimelineViewStore({ pxPerBeat: 20, scrollBeats: 10, widthPx: 800 });
    animateZoom(view, 2, 100);
    frames(2);
    animateZoom(view, 0.5, 500); // new anchor mid-flight
    frames(200);
    expect(view.getState().pxPerBeat).toBeCloseTo(20, 6);
  });

  it("pans by the accumulated delta, clamped at the song start", () => {
    const view = createTimelineViewStore({ pxPerBeat: 20, scrollBeats: 10, widthPx: 800 });
    animatePan(view, 100);
    animatePan(view, 100);
    frames(200);
    expect(view.getState().scrollBeats).toBeCloseTo(20, 6);
    animatePan(view, -10_000);
    frames(200);
    expect(view.getState().scrollBeats).toBe(0);
  });

  it("stops when something else moves the view", () => {
    const view = createTimelineViewStore({ pxPerBeat: 20, scrollBeats: 10, widthPx: 800 });
    animatePan(view, 1000);
    frames(2);
    view.getState().scrollTo(3);
    frames(200);
    expect(view.getState().scrollBeats).toBe(3);
    animatePan(view, 200); // starts from the new position
    frames(200);
    expect(view.getState().scrollBeats).toBeCloseTo(13, 6);
  });
});

describe("followers", () => {
  it("ScaleFollower applies the pushed factors incrementally, within bounds", () => {
    let scale = 1;
    const f = new ScaleFollower((k) => (scale *= k));
    f.push(2);
    f.push(2);
    frames(200);
    expect(scale).toBeCloseTo(4, 6);
    f.push(100, { min: 0.5, max: 2 });
    frames(200);
    expect(scale).toBeCloseTo(8, 6);
  });

  it("ScrollYFollower animates scrollTop within the scrollable range", () => {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollHeight", { value: 1000 });
    Object.defineProperty(el, "clientHeight", { value: 200 });
    const s = new ScrollYFollower(el);
    s.scrollBy(300);
    frames(2);
    expect(el.scrollTop).toBeGreaterThan(0);
    expect(el.scrollTop).toBeLessThan(300);
    s.scrollBy(10_000);
    frames(200);
    expect(el.scrollTop).toBe(800);
  });
});

describe("animation level (Settings > Appearance)", () => {
  afterEach(() => setMotionLevel("classic"));

  it("fast halves the springs and tweens, reduced turns motion off; both reach CSS and are remembered", () => {
    setMotionLevel("classic");
    const zoom = MOTION.zoomHalfLife;
    setMotionLevel("fast");
    expect(MOTION.enabled).toBe(true);
    expect(MOTION.speed).toBe(0.5);
    expect(MOTION.zoomHalfLife).toBeCloseTo(zoom / 2, 9);
    expect(document.documentElement.dataset.motion).toBe("fast");
    expect(localStorage.getItem("eth-motion")).toBe("fast");
    setMotionLevel("reduced");
    expect(MOTION.enabled).toBe(false);
    expect(document.documentElement.dataset.motion).toBe("reduced");
    expect(getMotionLevel()).toBe("reduced");
  });
});
