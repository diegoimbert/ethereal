/**
 * Theme runtime: typed token access + theme switching. Values live in `tokens.ts`; CSS
 * custom properties in the generated `tokens.css` (imported via `@/kit`).
 *
 * In feature code prefer CSS (`var(--eth-color-accent)`); use `cssVar()` in inline styles and
 * the raw values (e.g. `TRACK_COLORS`, `readToken()`) only where CSS vars don't reach (canvas).
 */
import { useMemo, useSyncExternalStore } from "react";
import { darkColors, DEFAULT_THEME, THEME_NAMES, themes, tokenName, TRACK_COLORS, type ColorKey, type ThemeName } from "./tokens";

export * from "./tokens";

/** Dark-theme color values (kept for backwards compatibility; theme-aware code uses `readToken`). */
export const colors = darkColors;

/** `var(--eth-color-<key>)` for use in inline styles. */
export function cssVar(key: ColorKey): string {
  return `var(${tokenName("color", key)})`;
}

export function trackColor(index: number): string {
  const n = TRACK_COLORS.length;
  return TRACK_COLORS[((index % n) + n) % n] as string;
}

/** Computed value of a custom property on `el` (default: `<html>`), e.g. for canvas drawing. */
export function readToken(name: `--${string}`, el?: Element): string {
  if (typeof document === "undefined") return "";
  return getComputedStyle(el ?? document.documentElement)
    .getPropertyValue(name)
    .trim();
}

// ---- Theme switching ------------------------------------------------------------------

const STORAGE_KEY = "eth-theme";
const listeners = new Set<() => void>();

function isTheme(v: unknown): v is ThemeName {
  return typeof v === "string" && (THEME_NAMES as string[]).includes(v);
}

function storedTheme(): ThemeName | null {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return isTheme(v) ? v : null;
  } catch {
    return null;
  }
}

/** The theme currently applied to `<html data-theme>`. */
export function getTheme(): ThemeName {
  if (typeof document === "undefined") return DEFAULT_THEME;
  const t = document.documentElement.dataset.theme;
  return isTheme(t) ? t : DEFAULT_THEME;
}

/** Applies `theme` to `<html data-theme>` and remembers it (localStorage). */
export function setTheme(theme: ThemeName): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // storage unavailable (private mode, sandbox): the theme still applies for this session
  }
  themeColorCache.clear();
  listeners.forEach((l) => l());
}

/** Applies the remembered theme (or the default). Called once when `@/kit` is imported. */
export function initTheme(): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = storedTheme() ?? DEFAULT_THEME;
  try {
    if (localStorage.getItem(TRANSLUCENCY_KEY) === "off") document.documentElement.dataset.translucency = "off";
  } catch {
    // storage unavailable: translucent (the default)
  }
  const stored = storedAmount();
  if (stored !== null) {
    amount = stored;
    applyAmount(stored);
  }
  try {
    if (localStorage.getItem(COLOR_BLOBS_KEY) === "off") document.documentElement.dataset.blobs = "off";
    if (localStorage.getItem(GLOW_KEY) === "off") document.documentElement.dataset.glow = "off";
  } catch {
    // storage unavailable: shown (the default)
  }
  zoom = storedZoom() ?? ZOOM_DEFAULT;
  if (zoom !== ZOOM_DEFAULT) applyZoom(zoom);
}

const COLOR_BLOBS_KEY = "eth-color-blobs";
const GLOW_KEY = "eth-glow";

/**
 * Whether selections glow (Settings > Appearance): selected notes and clips, the selected
 * track's card, the playhead and status dots. Off: `<html data-glow="off">`, and the canvases
 * (arranger, piano roll) leave their halos out.
 */
export function getGlow(): boolean {
  if (typeof document === "undefined") return true;
  return document.documentElement.dataset.glow !== "off";
}

/** Turns the glows on or off (`<html data-glow>`) and remembers it (localStorage). */
export function setGlow(on: boolean): void {
  if (typeof document === "undefined") return;
  if (on) delete document.documentElement.dataset.glow;
  else document.documentElement.dataset.glow = "off";
  try {
    localStorage.setItem(GLOW_KEY, on ? "on" : "off");
  } catch {
    // storage unavailable: applies for this session
  }
  listeners.forEach((l) => l());
}

/** The glow setting, re-rendering on change. */
export function useGlow(): [boolean, (on: boolean) => void] {
  const on = useSyncExternalStore(subscribe, getGlow, () => true);
  return [on, setGlow];
}

/**
 * Whether the drifting color blobs are shown (Settings > Appearance): behind modals and,
 * faintly, behind the arrangement.
 */
export function getColorBlobs(): boolean {
  if (typeof document === "undefined") return true;
  return document.documentElement.dataset.blobs !== "off";
}

/** Shows or hides them (`<html data-blobs>`) and remembers it (localStorage). */
export function setColorBlobs(on: boolean): void {
  if (typeof document === "undefined") return;
  if (on) delete document.documentElement.dataset.blobs;
  else document.documentElement.dataset.blobs = "off";
  try {
    localStorage.setItem(COLOR_BLOBS_KEY, on ? "on" : "off");
  } catch {
    // storage unavailable: applies for this session
  }
  listeners.forEach((l) => l());
}

/** The color blobs setting, re-rendering on change. */
export function useColorBlobs(): [boolean, (on: boolean) => void] {
  const on = useSyncExternalStore(subscribe, getColorBlobs, () => true);
  return [on, setColorBlobs];
}

const ZOOM_KEY = "eth-ui-zoom";
/** Interface zoom (%) range and default (Settings > Appearance). */
export const ZOOM_MIN = 70;
export const ZOOM_MAX = 150;
export const ZOOM_DEFAULT = 100;

function storedZoom(): number | null {
  try {
    const raw = localStorage.getItem(ZOOM_KEY);
    const v = Number(raw);
    return raw !== null && Number.isFinite(v) ? Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, v)) : null;
  } catch {
    return null;
  }
}

/**
 * Scales the whole interface. The desktop app zooms its webview (like a browser's page zoom:
 * canvases and pointer positions stay exact); a browser gets CSS `zoom` on the page.
 */
function applyZoom(pct: number): void {
  const factor = pct / 100;
  if ("__TAURI_INTERNALS__" in window) {
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) => getCurrentWebview().setZoom(factor))
      .catch((e: unknown) => console.warn("Ethereal: setting the zoom failed", e));
    return;
  }
  document.documentElement.style.zoom = pct === ZOOM_DEFAULT ? "" : String(factor);
}

let zoom = ZOOM_DEFAULT;

/** The interface zoom, in %. */
export function getZoom(): number {
  return zoom;
}

/** Applies an interface zoom of `pct` % and remembers it (localStorage). */
export function setZoom(pct: number): void {
  if (typeof document === "undefined") return;
  zoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, Math.round(pct)));
  applyZoom(zoom);
  try {
    localStorage.setItem(ZOOM_KEY, String(zoom));
  } catch {
    // storage unavailable: applies for this session
  }
  listeners.forEach((l) => l());
}

/** The interface zoom (%), re-rendering on change. */
export function useZoom(): [number, (pct: number) => void] {
  const pct = useSyncExternalStore(subscribe, getZoom, () => ZOOM_DEFAULT);
  return [pct, setZoom];
}

const TRANSLUCENCY_KEY = "eth-translucency";

/** Whether dialogs and floating panes are translucent (blurred); off for slow machines. */
export function getTranslucency(): boolean {
  if (typeof document === "undefined") return true;
  return document.documentElement.dataset.translucency !== "off";
}

/** Applies `on` to `<html data-translucency>` and remembers it (localStorage). */
export function setTranslucency(on: boolean): void {
  if (typeof document === "undefined") return;
  if (on) delete document.documentElement.dataset.translucency;
  else document.documentElement.dataset.translucency = "off";
  try {
    localStorage.setItem(TRANSLUCENCY_KEY, on ? "on" : "off");
  } catch {
    // storage unavailable: applies for this session
  }
  listeners.forEach((l) => l());
}

const AMOUNT_KEY = "eth-translucency-amount";
/** Translucency (%, how see-through) range and default (the `opacity.translucentSurface` token). */
export const TRANSLUCENCY_MAX = 60;
export const TRANSLUCENCY_DEFAULT = 24;

function storedAmount(): number | null {
  try {
    const v = Number(localStorage.getItem(AMOUNT_KEY));
    return localStorage.getItem(AMOUNT_KEY) !== null && Number.isFinite(v) ? Math.max(0, Math.min(TRANSLUCENCY_MAX, v)) : null;
  } catch {
    return null;
  }
}

function applyAmount(pct: number): void {
  document.documentElement.style.setProperty("--eth-opacity-translucent-surface", String(1 - pct / 100));
}

let amount = TRANSLUCENCY_DEFAULT;

/** How see-through dialogs and floating panes are, in % (0 = opaque). */
export function getTranslucencyAmount(): number {
  return amount;
}

/** Applies a translucency of `pct` % and remembers it (localStorage). */
export function setTranslucencyAmount(pct: number): void {
  if (typeof document === "undefined") return;
  amount = Math.max(0, Math.min(TRANSLUCENCY_MAX, Math.round(pct)));
  applyAmount(amount);
  try {
    localStorage.setItem(AMOUNT_KEY, String(amount));
  } catch {
    // storage unavailable: applies for this session
  }
  listeners.forEach((l) => l());
}

/** The translucency amount (%), re-rendering on change. */
export function useTranslucencyAmount(): [number, (pct: number) => void] {
  const pct = useSyncExternalStore(subscribe, getTranslucencyAmount, () => TRANSLUCENCY_DEFAULT);
  return [pct, setTranslucencyAmount];
}

/** The translucency setting, re-rendering on change. */
export function useTranslucency(): [boolean, (on: boolean) => void] {
  const on = useSyncExternalStore(subscribe, getTranslucency, () => true);
  return [on, setTranslucency];
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

/** React hook: `[theme, setTheme]`. */
export function useTheme(): [ThemeName, (t: ThemeName) => void] {
  const theme = useSyncExternalStore(subscribe, getTheme, () => DEFAULT_THEME);
  return [theme, setTheme];
}

/**
 * A color token's current value, for canvas drawing (CSS vars don't reach a canvas). Read from
 * the page (so a restyled `tokens.css` or a devtools override applies), falling back to the
 * theme's value in `tokens.ts` where no stylesheet is loaded (tests). Re-renders the caller
 * when the theme changes: put the value in the canvas's redraw dependencies.
 */
export function useThemeColor(key: ColorKey): string {
  const [theme] = useTheme();
  return useMemo(() => themeColor(theme, key), [key, theme]);
}

/**
 * Resolved theme colors, read from the page once per theme and key: `readToken` calls
 * `getComputedStyle`, which forces a synchronous style recalc of the whole page, and canvas
 * components mount by the hundred while zooming (clips switching level of detail).
 */
const themeColorCache = new Map<string, string>();

function themeColor(theme: ThemeName, key: ColorKey): string {
  const id = `${theme}:${key}`;
  let value = themeColorCache.get(id);
  if (value === undefined) {
    value = readToken(tokenName("color", key) as `--${string}`) || themes[theme].color[key];
    themeColorCache.set(id, value);
  }
  return value;
}
