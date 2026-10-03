/**
 * Wheel input normalization and the per-device input settings (base-135).
 *
 * Browsers report wheels very differently:
 * - macOS trackpads (and macOS mice, which the OS smooths) send many small pixel deltas;
 * - Windows/Linux mice in Chromium (Tauri's WebView2 included) send one large pixel delta
 *   per notch (100 px at 100% scaling, more when scaled), with the legacy `wheelDelta`
 *   a multiple of 120;
 * - Firefox mice send `deltaMode = LINE` (usually 3 lines per notch).
 *
 * `normalizeWheel` turns any of these into pixels and flags discrete mouse notches. Smooth
 * input keeps its exact pixels (and the exact zoom feel it always had); a notch counts as
 * `NOTCH_PX` pixels and zooms by a fixed `NOTCH_ZOOM` factor. The user's settings
 * (sensitivity, inversion, which modifier zooms) then apply in `wheelZoomFactorFor` and
 * `wheelScrollPx`.
 */

import { MOTION } from "./motion";

export type ZoomModifier = "ctrlCmd" | "alt";
export type PlainWheel = "scroll" | "zoom";
export type MiddleButtonAction = "pan" | "off";
export type SideButtonAction = "none" | "undoRedo";
export type Platform = "mac" | "windows" | "linux" | "other";

export interface InputSettings {
  /** Zoom speed multiplier (1 = default). */
  zoomSensitivity: number;
  /** Scroll speed multiplier (1 = default). */
  scrollSensitivity: number;
  /** Flip the zoom direction of wheels and two-finger swipes (pinch is never flipped). */
  invertZoom: boolean;
  /** Flip vertical scrolling (track list, piano roll keys, value axes, wheel-adjusted controls). */
  invertScrollY: boolean;
  /** Flip horizontal scrolling (the timeline). */
  invertScrollX: boolean;
  /** What a wheel without the zoom modifier does. With "zoom", the modifier scrolls instead. */
  plainWheel: PlainWheel;
  /** The modifier that switches the wheel between scroll and zoom. */
  zoomModifier: ZoomModifier;
  /** Shift + wheel scrolls the timeline horizontally. */
  shiftScrollsHorizontally: boolean;
  /** Middle-button drag. */
  middleButton: MiddleButtonAction;
  /** Mouse side buttons (Back / Forward). */
  sideButtons: SideButtonAction;
}

/** Pixels one mouse-wheel notch scrolls (what Chromium reports on Windows at 100% scaling). */
export const NOTCH_PX = 100;
/** Pixels per line for `deltaMode = LINE` (browsers send 3 lines per notch). */
export const LINE_PX = NOTCH_PX / 3;
/** Zoom per mouse-wheel notch at sensitivity 1: ×1.12 (12%). */
export const NOTCH_ZOOM = 1.12;

export const SENSITIVITY_MIN = 0.25;
export const SENSITIVITY_MAX = 4;

const BASE: InputSettings = {
  zoomSensitivity: 1,
  scrollSensitivity: 1,
  invertZoom: false,
  invertScrollY: false,
  invertScrollX: false,
  plainWheel: "scroll",
  zoomModifier: "ctrlCmd",
  shiftScrollsHorizontally: true,
  middleButton: "pan",
  sideButtons: "none",
};

/**
 * Defaults per platform, so a Windows mouse feels like the Mac out of the box. On the Mac
 * (natural scrolling) pushing the wheel / swiping up with Cmd zooms out; a Windows wheel
 * pushed forward reports the opposite sign, so Windows flips zoom by default.
 */
export function defaultInputSettings(platform: Platform): InputSettings {
  return { ...BASE, invertZoom: platform === "windows" };
}

export function detectPlatform(): Platform {
  if (typeof navigator === "undefined") return "other";
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  // The OS (`platform`) first: the user agent and its client hints are often spoofed (device
  // emulation reports a Windows UA on any host), `platform` isn't.
  const p = (nav.platform || nav.userAgentData?.platform || nav.userAgent || "").toLowerCase();
  if (p.includes("win")) return "windows";
  if (p.includes("mac")) return "mac";
  if (p.includes("linux") || p.includes("x11")) return "linux";
  return "other";
}

/** Settings read from storage: unknown keys dropped, invalid values replaced by defaults. */
export function sanitizeInputSettings(raw: unknown, defaults: InputSettings): InputSettings {
  const v = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const num = (k: "zoomSensitivity" | "scrollSensitivity") => {
    const x = v[k];
    return typeof x === "number" && Number.isFinite(x) ? clampSensitivity(x) : defaults[k];
  };
  const bool = (k: "invertZoom" | "invertScrollY" | "invertScrollX" | "shiftScrollsHorizontally") =>
    typeof v[k] === "boolean" ? (v[k] as boolean) : defaults[k];
  const oneOf = <T extends string>(k: keyof InputSettings, values: readonly T[], d: T): T =>
    values.includes(v[k] as T) ? (v[k] as T) : d;
  return {
    zoomSensitivity: num("zoomSensitivity"),
    scrollSensitivity: num("scrollSensitivity"),
    invertZoom: bool("invertZoom"),
    invertScrollY: bool("invertScrollY"),
    invertScrollX: bool("invertScrollX"),
    shiftScrollsHorizontally: bool("shiftScrollsHorizontally"),
    plainWheel: oneOf("plainWheel", ["scroll", "zoom"] as const, defaults.plainWheel),
    zoomModifier: oneOf("zoomModifier", ["ctrlCmd", "alt"] as const, defaults.zoomModifier),
    middleButton: oneOf("middleButton", ["pan", "off"] as const, defaults.middleButton),
    sideButtons: oneOf("sideButtons", ["none", "undoRedo"] as const, defaults.sideButtons),
  };
}

export function clampSensitivity(x: number): number {
  return Math.min(SENSITIVITY_MAX, Math.max(SENSITIVITY_MIN, x));
}

/** The fields of a `WheelEvent` the normalization reads (`wheelDelta*` are legacy, non-standard). */
export interface WheelSample {
  deltaX: number;
  deltaY: number;
  deltaMode: number;
  wheelDeltaX?: number;
  wheelDeltaY?: number;
}

export interface NormalizedWheel {
  /** Horizontal delta in px (positive = right / later). */
  dx: number;
  /** Vertical delta in px (positive = down). */
  dy: number;
  /** A discrete mouse-wheel step (notch) rather than smooth trackpad input. */
  discrete: boolean;
}

const LINE = 1;
const PAGE = 2;

/** One axis of a legacy `wheelDelta` is a notch step: a multiple of 120 that isn't 3× the pixels (WebKit/Blink trackpads). */
function legacyNotch(delta: number, legacy: number | undefined): boolean {
  if (!legacy || !delta) return false;
  if (Math.abs(legacy) % 120 !== 0) return false;
  return Math.abs(Math.abs(legacy) - 3 * Math.abs(delta)) > 1;
}

/**
 * Whether the event is a discrete mouse-wheel notch: `deltaMode` LINE/PAGE (Firefox), or
 * pixels with a legacy `wheelDelta` in whole 120-unit steps (Chromium on Windows/Linux).
 * Smooth trackpads, and every macOS wheel (the OS smooths them, and WebKit/Blink report
 * `wheelDelta = -3 × delta`), are not.
 */
export function isDiscreteWheel(e: WheelSample): boolean {
  if (e.deltaMode === LINE || e.deltaMode === PAGE) return true;
  return legacyNotch(e.deltaY, e.wheelDeltaY) || legacyNotch(e.deltaX, e.wheelDeltaX);
}

/**
 * Pixels of an event. A notch scrolls `NOTCH_PX` (whatever the browser's own step), lines
 * are `LINE_PX`, pages are `pagePx`. Smooth input keeps its pixels untouched.
 */
export function normalizeWheel(e: WheelSample, pagePx: { width: number; height: number } = { width: 800, height: 600 }): NormalizedWheel {
  if (e.deltaMode === LINE) return { dx: e.deltaX * LINE_PX, dy: e.deltaY * LINE_PX, discrete: true };
  if (e.deltaMode === PAGE) return { dx: e.deltaX * pagePx.width, dy: e.deltaY * pagePx.height, discrete: true };
  if (isDiscreteWheel(e)) {
    // Whole notches from wheelDelta (the pixel step varies with display scaling), unless the
    // pixels say clearly more (synthetic/automation events cap wheelDelta at one notch).
    const notches = (delta: number, legacy: number | undefined) => {
      if (!legacy || Math.abs(legacy) % 120 !== 0) return delta / NOTCH_PX;
      const n = Math.abs(legacy) / 120;
      return Math.sign(delta) * (Math.abs(delta) > 1.6 * NOTCH_PX * n ? Math.abs(delta) / NOTCH_PX : n);
    };
    return { dx: notches(e.deltaX, e.wheelDeltaX) * NOTCH_PX, dy: notches(e.deltaY, e.wheelDeltaY) * NOTCH_PX, discrete: true };
  }
  return { dx: e.deltaX, dy: e.deltaY, discrete: false };
}

/** Zoom (log factor) per px of delta: notches zoom by `NOTCH_ZOOM` each; smooth input as always. */
function zoomPerPx(discrete: boolean): number {
  return discrete ? Math.log(NOTCH_ZOOM) / NOTCH_PX : MOTION.wheelZoomSensitivity;
}

/**
 * Zoom factor (> 1 = zoom in) for a normalized wheel delta `d` (px, positive = down):
 * exponential, so one notch in then one out returns to the same zoom. `pinch` (a trackpad
 * pinch, reported as ctrl + wheel) is never inverted.
 */
export function wheelZoomFactorFor(
  d: number,
  discrete: boolean,
  settings: Pick<InputSettings, "zoomSensitivity" | "invertZoom">,
  pinch = false,
): number {
  const sign = settings.invertZoom && !pinch ? -1 : 1;
  return Math.exp(-sign * d * zoomPerPx(discrete) * settings.zoomSensitivity);
}

/** Scroll amount in px for a normalized delta on `axis`, with the user's sensitivity and inversion. */
export function wheelScrollPx(
  d: number,
  axis: "x" | "y",
  settings: Pick<InputSettings, "scrollSensitivity" | "invertScrollX" | "invertScrollY">,
): number {
  const invert = axis === "x" ? settings.invertScrollX : settings.invertScrollY;
  return (invert ? -d : d) * settings.scrollSensitivity;
}

/** What a wheel event asks for, given the modifiers held and the settings. */
export type WheelIntent = "zoom" | "verticalZoom" | "scrollX" | "scrollY";

export interface WheelModifiers {
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  /** A trackpad pinch (ctrl + wheel with no Ctrl key physically down). */
  pinch: boolean;
}

/** Whether the configured zoom modifier is held (a pinch's synthetic ctrl doesn't count). */
export function zoomModifierHeld(m: WheelModifiers, s: Pick<InputSettings, "zoomModifier">): boolean {
  if (s.zoomModifier === "alt") return m.altKey;
  return m.metaKey || (m.ctrlKey && !m.pinch);
}

/**
 * Classify a wheel event. Pinch always zooms. The zoom modifier + Shift zooms vertically
 * (when the view supports it). Otherwise the modifier swaps between scroll and zoom per
 * `plainWheel`; Shift (when enabled) or a mostly horizontal delta scrolls horizontally.
 */
export function wheelIntent(
  m: WheelModifiers,
  n: Pick<NormalizedWheel, "dx" | "dy">,
  s: Pick<InputSettings, "zoomModifier" | "plainWheel" | "shiftScrollsHorizontally">,
  hasVerticalZoom: boolean,
): WheelIntent {
  if (m.pinch) return "zoom";
  const mod = zoomModifierHeld(m, s);
  if (mod && m.shiftKey && hasVerticalZoom) return "verticalZoom";
  const shiftScroll = m.shiftKey && s.shiftScrollsHorizontally;
  const wantsZoom = s.plainWheel === "zoom" ? !mod && !shiftScroll : mod;
  if (wantsZoom) return "zoom";
  if (shiftScroll || Math.abs(n.dx) > Math.abs(n.dy)) return "scrollX";
  return "scrollY";
}
