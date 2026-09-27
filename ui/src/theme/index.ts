/**
 * Theme runtime: typed token access + theme switching. Values live in `tokens.ts`; CSS
 * custom properties in the generated `tokens.css` (imported via `@/kit`).
 *
 * In feature code prefer CSS (`var(--eth-color-accent)`); use `cssVar()` in inline styles and
 * the raw values (e.g. `TRACK_COLORS`, `readToken()`) only where CSS vars don't reach (canvas).
 */
import { useSyncExternalStore } from "react";
import { darkColors, DEFAULT_THEME, THEME_NAMES, tokenName, TRACK_COLORS, type ColorKey, type ThemeName } from "./tokens";

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
  listeners.forEach((l) => l());
}

/** Applies the remembered theme (or the default). Called once when `@/kit` is imported. */
export function initTheme(): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = storedTheme() ?? DEFAULT_THEME;
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
