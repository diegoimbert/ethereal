import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useInputSettings } from "./inputSettings";
import { useTimelineWheel, zoomAnchorPx } from "./useTimelineWheel";
import { createTimelineViewStore } from "./viewStore";
import { NOTCH_ZOOM } from "./wheelInput";

describe("zoomAnchorPx", () => {
  it("anchors on the pointer over the timeline", () => {
    expect(zoomAnchorPx(500, 200)).toBe(300);
  });
  it("clamps to the timeline's left edge over the left pane", () => {
    expect(zoomAnchorPx(120, 200)).toBe(0);
    expect(zoomAnchorPx(0, 200)).toBe(0);
  });
});

describe("useTimelineWheel with the input settings", () => {
  afterEach(() => act(() => useInputSettings.getState().reset()));

  function setup() {
    const el = document.createElement("div");
    document.body.appendChild(el);
    const view = createTimelineViewStore({ pxPerBeat: 20 });
    view.getState().setWidth(1000);
    renderHook(() => useTimelineWheel({ current: el }, view));
    /** A Windows-like wheel notch (Firefox: LINE mode, 3 lines) with Ctrl physically held. */
    const notch = (lines: number, init: WheelEventInit = { ctrlKey: true }) => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Control", ctrlKey: !!init.ctrlKey }));
      el.dispatchEvent(new WheelEvent("wheel", { deltaY: lines, deltaMode: 1, bubbles: true, cancelable: true, ...init }));
      window.dispatchEvent(new KeyboardEvent("keyup", { key: "Control", ctrlKey: false }));
    };
    return { view, notch, zoom: () => view.getState().pxPerBeat, scroll: () => view.getState().scrollBeats };
  }

  it("zooms by the per-notch factor, symmetric in and out", () => {
    const { notch, zoom } = setup();
    notch(-3);
    expect(zoom()).toBeCloseTo(20 * NOTCH_ZOOM);
    notch(3);
    expect(zoom()).toBeCloseTo(20);
  });

  it("follows invert zoom and zoom sensitivity", () => {
    const { notch, zoom } = setup();
    act(() => useInputSettings.getState().set({ invertZoom: true, zoomSensitivity: 2 }));
    notch(-3);
    expect(zoom()).toBeCloseTo(20 / NOTCH_ZOOM ** 2);
  });

  it("Alt as the zoom modifier; plain wheel zooms when set", () => {
    const { notch, zoom } = setup();
    act(() => useInputSettings.getState().set({ zoomModifier: "alt" }));
    notch(-3, { altKey: true });
    expect(zoom()).toBeCloseTo(20 * NOTCH_ZOOM);
    act(() => useInputSettings.getState().set({ zoomModifier: "ctrlCmd", plainWheel: "zoom" }));
    notch(-3, {});
    expect(zoom()).toBeCloseTo(20 * NOTCH_ZOOM ** 2);
  });

  it("Shift + wheel scrolls horizontally, inverted when set", () => {
    const { notch, scroll, view } = setup();
    view.getState().scrollTo(10);
    notch(3, { shiftKey: true });
    const right = scroll();
    expect(right).toBeGreaterThan(10);
    act(() => useInputSettings.getState().set({ invertScrollX: true }));
    notch(3, { shiftKey: true });
    expect(scroll()).toBeCloseTo(10);
  });
});
