import { describe, expect, it, vi } from "vitest";
import { blinkEpoch, blinkRef, restartBlink } from "./blink";

/** An element with one fake (CSS) animation, as `getAnimations` returns it. */
function marker() {
  const anim = { startTime: null as number | null };
  const el = document.createElement("div");
  (el as unknown as { getAnimations: () => unknown[] }).getAnimations = () => [anim];
  return { el, anim };
}

describe("shared blink phase (#238)", () => {
  it("markers mounted at different times blink from one start", () => {
    const spy = vi.spyOn(performance, "now");
    spy.mockReturnValue(1000);
    const a = marker();
    const cleanupA = blinkRef(a.el);
    spy.mockReturnValue(1730);
    const b = marker();
    const cleanupB = blinkRef(b.el);
    expect(a.anim.startTime).toBe(blinkEpoch());
    expect(b.anim.startTime).toBe(a.anim.startTime);
    cleanupA?.();
    cleanupB?.();
    spy.mockRestore();
  });

  it("restarting the phase realigns every live marker to now", () => {
    const spy = vi.spyOn(performance, "now");
    spy.mockReturnValue(5000);
    const a = marker();
    const b = marker();
    const cleanupA = blinkRef(a.el);
    const cleanupB = blinkRef(b.el);
    spy.mockReturnValue(9000);
    restartBlink();
    expect(a.anim.startTime).toBe(9000);
    expect(b.anim.startTime).toBe(9000);
    // An unmounted marker is no longer touched.
    cleanupB?.();
    spy.mockReturnValue(9500);
    restartBlink();
    expect(a.anim.startTime).toBe(9500);
    expect(b.anim.startTime).toBe(9000);
    cleanupA?.();
    spy.mockRestore();
  });

  it("tolerates elements without Web Animations (jsdom)", () => {
    const el = document.createElement("div");
    expect(() => blinkRef(el)?.()).not.toThrow();
  });
});
