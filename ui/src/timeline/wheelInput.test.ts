import { describe, expect, it } from "vitest";
import { MOTION } from "./motion";
import {
  ZoomAccelerator,
  defaultInputSettings,
  detectPlatform,
  isDiscreteWheel,
  LINE_PX,
  normalizeWheel,
  NOTCH_PX,
  NOTCH_ZOOM,
  sanitizeInputSettings,
  wheelIntent,
  wheelScrollPx,
  wheelZoomFactorFor,
  type InputSettings,
} from "./wheelInput";

const base = defaultInputSettings("mac");
const s = (patch: Partial<InputSettings> = {}): InputSettings => ({ ...base, ...patch });
const PIXEL = 0;
const LINE = 1;
const PAGE = 2;

describe("normalizeWheel", () => {
  it("converts LINE mode to pixels (3 lines = one notch) and flags it discrete", () => {
    expect(normalizeWheel({ deltaX: 0, deltaY: 3, deltaMode: LINE })).toEqual({ dx: 0, dy: 3 * LINE_PX, discrete: true });
    expect(3 * LINE_PX).toBeCloseTo(NOTCH_PX);
  });

  it("converts PAGE mode to the element's size", () => {
    expect(normalizeWheel({ deltaX: 1, deltaY: -1, deltaMode: PAGE }, { width: 500, height: 300 })).toEqual({
      dx: 500,
      dy: -300,
      discrete: true,
    });
  });

  it("keeps smooth pixel deltas untouched (macOS trackpad)", () => {
    expect(normalizeWheel({ deltaX: 0.5, deltaY: 4.25, deltaMode: PIXEL, wheelDeltaY: -12 })).toEqual({
      dx: 0.5,
      dy: 4.25,
      discrete: false,
    });
    // No legacy wheelDelta at all: treated as smooth.
    expect(normalizeWheel({ deltaX: 0, deltaY: 100, deltaMode: PIXEL }).discrete).toBe(false);
  });

  it("counts Chromium mouse notches from wheelDelta, whatever the pixel step (display scaling)", () => {
    expect(normalizeWheel({ deltaX: 0, deltaY: 100, deltaMode: PIXEL, wheelDeltaY: -120 })).toEqual({
      dx: 0,
      dy: NOTCH_PX,
      discrete: true,
    });
    expect(normalizeWheel({ deltaX: 0, deltaY: -150, deltaMode: PIXEL, wheelDeltaY: 120 }).dy).toBe(-NOTCH_PX);
    expect(normalizeWheel({ deltaX: 0, deltaY: -250, deltaMode: PIXEL, wheelDeltaY: 240 }).dy).toBe(-2 * NOTCH_PX);
    // wheelDelta capped at one notch (automation) while the pixels say three: trust the pixels.
    expect(normalizeWheel({ deltaX: 0, deltaY: -300, deltaMode: PIXEL, wheelDeltaY: 120 }).dy).toBe(-3 * NOTCH_PX);
  });
});

describe("isDiscreteWheel (notch detection)", () => {
  it("flags LINE/PAGE modes and 120-unit legacy steps", () => {
    expect(isDiscreteWheel({ deltaX: 0, deltaY: 3, deltaMode: LINE })).toBe(true);
    expect(isDiscreteWheel({ deltaX: 0, deltaY: 1, deltaMode: PAGE })).toBe(true);
    expect(isDiscreteWheel({ deltaX: 0, deltaY: -100, deltaMode: PIXEL, wheelDeltaY: 120 })).toBe(true);
    expect(isDiscreteWheel({ deltaX: 100, deltaY: 0, deltaMode: PIXEL, wheelDeltaX: -120 })).toBe(true);
  });

  it("does not flag WebKit/Blink trackpads and macOS wheels (wheelDelta = -3 × delta)", () => {
    expect(isDiscreteWheel({ deltaX: 0, deltaY: 40, deltaMode: PIXEL, wheelDeltaY: -120 })).toBe(false);
    expect(isDiscreteWheel({ deltaX: 0, deltaY: 2.5, deltaMode: PIXEL, wheelDeltaY: -7 })).toBe(false);
    // High-resolution / free-spinning wheels: not whole notches, so smooth.
    expect(isDiscreteWheel({ deltaX: 0, deltaY: 12.5, deltaMode: PIXEL, wheelDeltaY: -15 })).toBe(false);
  });
});

describe("wheelZoomFactorFor", () => {
  it("zooms by NOTCH_ZOOM per notch at sensitivity 1 (LINE or PIXEL notches alike)", () => {
    const line = normalizeWheel({ deltaX: 0, deltaY: -3, deltaMode: LINE });
    const px = normalizeWheel({ deltaX: 0, deltaY: -100, deltaMode: PIXEL, wheelDeltaY: 120 });
    expect(wheelZoomFactorFor(line.dy, line.discrete, s())).toBeCloseTo(NOTCH_ZOOM);
    expect(wheelZoomFactorFor(px.dy, px.discrete, s())).toBeCloseTo(NOTCH_ZOOM);
    // Down = out.
    expect(wheelZoomFactorFor(NOTCH_PX, true, s())).toBeCloseTo(1 / NOTCH_ZOOM);
  });

  it("keeps the smooth (trackpad) feel exactly as before", () => {
    for (const d of [-37.5, -4, 0.25, 12]) {
      expect(wheelZoomFactorFor(d, false, s())).toBe(Math.exp(-d * MOTION.wheelZoomSensitivity));
    }
  });

  it("is symmetric: one notch in then one out returns to the same zoom", () => {
    for (const sens of [0.5, 1, 2.75]) {
      for (const invertZoom of [false, true]) {
        const set = s({ zoomSensitivity: sens, invertZoom });
        const z = 37 * wheelZoomFactorFor(-NOTCH_PX, true, set) * wheelZoomFactorFor(NOTCH_PX, true, set);
        expect(z).toBeCloseTo(37, 10);
      }
    }
  });

  it("scales the exponent with the sensitivity", () => {
    expect(wheelZoomFactorFor(-NOTCH_PX, true, s({ zoomSensitivity: 2 }))).toBeCloseTo(NOTCH_ZOOM ** 2);
    expect(wheelZoomFactorFor(-NOTCH_PX, true, s({ zoomSensitivity: 0.5 }))).toBeCloseTo(Math.sqrt(NOTCH_ZOOM));
  });

  it("inverts the direction, except for a pinch", () => {
    expect(wheelZoomFactorFor(-NOTCH_PX, true, s({ invertZoom: true }))).toBeCloseTo(1 / NOTCH_ZOOM);
    expect(wheelZoomFactorFor(-10, false, s({ invertZoom: true }), true)).toBe(wheelZoomFactorFor(-10, false, s(), true));
  });
});

describe("wheelScrollPx", () => {
  it("applies sensitivity and per-axis inversion", () => {
    expect(wheelScrollPx(100, "y", s())).toBe(100);
    expect(wheelScrollPx(100, "y", s({ scrollSensitivity: 1.5 }))).toBe(150);
    expect(wheelScrollPx(100, "y", s({ invertScrollY: true }))).toBe(-100);
    expect(wheelScrollPx(100, "x", s({ invertScrollY: true }))).toBe(100);
    expect(wheelScrollPx(-40, "x", s({ invertScrollX: true, scrollSensitivity: 0.5 }))).toBe(20);
  });
});

describe("wheelIntent", () => {
  const mods = { ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, pinch: false };
  const v = { dx: 0, dy: 100 };
  it("default: plain scrolls, Ctrl/Cmd zooms, Shift scrolls horizontally, pinch zooms", () => {
    expect(wheelIntent(mods, v, s(), true)).toBe("scrollY");
    expect(wheelIntent({ ...mods, ctrlKey: true }, v, s(), true)).toBe("zoom");
    expect(wheelIntent({ ...mods, metaKey: true }, v, s(), true)).toBe("zoom");
    expect(wheelIntent({ ...mods, metaKey: true, shiftKey: true }, v, s(), true)).toBe("verticalZoom");
    expect(wheelIntent({ ...mods, metaKey: true, shiftKey: true }, v, s(), false)).toBe("zoom");
    expect(wheelIntent({ ...mods, shiftKey: true }, v, s(), true)).toBe("scrollX");
    expect(wheelIntent(mods, { dx: 50, dy: 3 }, s(), true)).toBe("scrollX");
    expect(wheelIntent({ ...mods, ctrlKey: true, pinch: true }, v, s({ zoomModifier: "alt" }), true)).toBe("zoom");
  });
  it("Alt as the zoom modifier: Ctrl no longer zooms", () => {
    const alt = s({ zoomModifier: "alt" });
    expect(wheelIntent({ ...mods, altKey: true }, v, alt, true)).toBe("zoom");
    expect(wheelIntent({ ...mods, ctrlKey: true }, v, alt, true)).toBe("scrollY");
  });
  it("wheel zooms: plain zooms, the modifier scrolls, Shift still scrolls horizontally", () => {
    const zoom = s({ plainWheel: "zoom" });
    expect(wheelIntent(mods, v, zoom, true)).toBe("zoom");
    expect(wheelIntent({ ...mods, ctrlKey: true }, v, zoom, true)).toBe("scrollY");
    expect(wheelIntent({ ...mods, shiftKey: true }, v, zoom, true)).toBe("scrollX");
    expect(wheelIntent({ ...mods, shiftKey: true }, v, s({ plainWheel: "zoom", shiftScrollsHorizontally: false }), true)).toBe("zoom");
  });
});

describe("settings", () => {
  it("per-platform defaults: Windows flips zoom, everything else is the Mac's", () => {
    expect(defaultInputSettings("windows")).toEqual({ ...defaultInputSettings("mac"), invertZoom: true });
    expect(defaultInputSettings("linux")).toEqual(defaultInputSettings("mac"));
    expect(defaultInputSettings("mac")).toMatchObject({
      zoomSensitivity: 1,
      plainWheel: "scroll",
      zoomModifier: "ctrlCmd",
      middleButton: "pan",
      sideButtons: "none",
    });
  });
  it("sanitizes stored settings", () => {
    const d = defaultInputSettings("mac");
    expect(sanitizeInputSettings(null, d)).toEqual(d);
    expect(sanitizeInputSettings({ zoomSensitivity: 99, plainWheel: "nope", invertScrollX: true, extra: 1 }, d)).toEqual({
      ...d,
      zoomSensitivity: 4,
      invertScrollX: true,
    });
  });
});

describe("detectPlatform", () => {
  const withNavigator = (nav: Partial<Navigator>, run: () => void) => {
    const original = Object.getOwnPropertyDescriptor(globalThis, "navigator");
    Object.defineProperty(globalThis, "navigator", { value: nav, configurable: true });
    try {
      run();
    } finally {
      if (original) Object.defineProperty(globalThis, "navigator", original);
    }
  };

  it("reads the OS, the Mac before Windows ('darwin' contains 'win')", () => {
    withNavigator({ platform: "", userAgent: "Mozilla/5.0 (darwin) AppleWebKit/537.36 (KHTML, like Gecko) jsdom/26.1.0" }, () =>
      expect(detectPlatform()).toBe("mac"),
    );
    withNavigator({ platform: "MacIntel", userAgent: "" }, () => expect(detectPlatform()).toBe("mac"));
    withNavigator({ platform: "Win32", userAgent: "" }, () => expect(detectPlatform()).toBe("windows"));
    withNavigator({ platform: "Linux x86_64", userAgent: "" }, () => expect(detectPlatform()).toBe("linux"));
  });
});

describe("zoom acceleration", () => {
  it("leaves slow wheels as they are and multiplies fast ones, up to a cap; off at 0", () => {
    const slow = new ZoomAccelerator();
    // A notch every 200 ms: no acceleration.
    for (let i = 0; i < 5; i++) expect(slow.gain(100, i * 200, 1)).toBe(1);
    const fast = new ZoomAccelerator();
    // A notch every 20 ms: the gain grows with the speed, capped at ×4.
    const gains = Array.from({ length: 8 }, (_, i) => fast.gain(100, i * 20, 1));
    expect(gains[0]).toBe(1);
    expect(gains[3]!).toBeGreaterThan(2);
    expect(gains[7]).toBe(4);
    // Twice the acceleration setting, twice the extra.
    expect(new ZoomAccelerator().gain(1000, 0, 2)).toBe(7);
    expect(new ZoomAccelerator().gain(1000, 0, 0)).toBe(1);
  });

  it("starts over when the direction changes", () => {
    const a = new ZoomAccelerator();
    for (let i = 0; i < 6; i++) a.gain(100, i * 20, 1);
    expect(a.gain(-100, 130, 1)).toBe(1);
  });
});
