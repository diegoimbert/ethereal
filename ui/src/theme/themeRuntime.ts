// eth-allow-case-file: storage keys, attribute names and CSS text.
/**
 * Which theme is applied, the user's own themes, and following the system's light/dark.
 *
 * `<html data-theme>` holds the theme's scheme (`dark` / `light`: every light-only style rule
 * and most code key off it) and `<html data-theme-id>` the theme itself. Built-in themes are
 * declared in tokens.css; the user's own (and the theme editor's live preview) are written
 * into a `<style>` of their own. `data-theme-rev` changes with every applied change, so
 * observers of the attributes (the arranger's canvas) redraw even when only colors changed.
 *
 * Kept on this device (localStorage): `eth-theme` (the scheme, also read before the app's
 * CSS loads), `eth-theme-id`, `eth-theme-dark` / `eth-theme-light` (the theme used for each scheme: following the system,
 * the light/dark command), `eth-theme-mode` (`system` to follow it) and `eth-custom-themes`.
 */
import { BUILTIN_THEMES, builtinTheme, isBuiltinTheme, SCHEME_DEFAULT_THEME } from "./builtinThemes";
import { isThemeDefinition, resolvedThemeEntries, resolveTheme, type ResolvedTheme, type ThemeDefinition, type ThemeScheme } from "./themeModel";

const SCHEME_KEY = "eth-theme";
const ID_KEY = "eth-theme-id";
const PICK_KEY: Record<ThemeScheme, string> = { dark: "eth-theme-dark", light: "eth-theme-light" };
const MODE_KEY = "eth-theme-mode";
const CUSTOM_KEY = "eth-custom-themes";
const STYLE_ID = "eth-theme-custom";
/** `data-theme-id` of the theme editor's live preview. */
const PREVIEW_ID = "preview";

export type ThemeMode = "fixed" | "system";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // storage unavailable (private mode, sandbox): applies for this session
  }
}

let custom: ThemeDefinition[] = [];
let list: ThemeDefinition[] = [...BUILTIN_THEMES];
let currentId: string = SCHEME_DEFAULT_THEME.dark;
let pick: Record<ThemeScheme, string> = { ...SCHEME_DEFAULT_THEME };
let mode: ThemeMode = "fixed";
let preview: ThemeDefinition | null = null;
let rev = 0;
const resolvedCache = new Map<ThemeDefinition, ResolvedTheme>();
const listeners = new Set<() => void>();

function loadCustom(): ThemeDefinition[] {
  try {
    const raw: unknown = JSON.parse(read(CUSTOM_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter(isThemeDefinition).filter((d) => !isBuiltinTheme(d.id)) : [];
  } catch {
    return [];
  }
}

function setCustom(next: ThemeDefinition[], persist = true): void {
  custom = next;
  list = [...BUILTIN_THEMES, ...custom];
  if (persist) write(CUSTOM_KEY, JSON.stringify(custom));
}

/** Every theme: the built-ins, then the user's own. */
export function getThemes(): readonly ThemeDefinition[] {
  return list;
}

export function findTheme(id: string): ThemeDefinition | undefined {
  return list.find((d) => d.id === id);
}

/** `def` with every token resolved (cached per definition). */
export function resolved(def: ThemeDefinition): ResolvedTheme {
  if (isBuiltinTheme(def.id)) return builtinTheme(def.id);
  let r = resolvedCache.get(def);
  if (!r) resolvedCache.set(def, (r = resolveTheme(def)));
  return r;
}

function systemScheme(): ThemeScheme {
  if (typeof matchMedia === "undefined") return "dark";
  return matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

/** The theme on screen: the preview while the editor shows one, else the chosen theme. */
export function appliedTheme(): ResolvedTheme {
  return resolved(preview ?? findTheme(currentId) ?? BUILTIN_THEMES[0]);
}

/** The chosen theme's id (not the editor's preview). */
export function getThemeId(): string {
  return currentId;
}

/** Changes with every applied change (theme, preview, edited colors): a redraw key. */
export function getThemeKey(): string {
  return `${preview ? PREVIEW_ID : currentId}#${rev}`;
}

export function getThemeMode(): ThemeMode {
  return mode;
}

/** The theme used for `scheme` (following the system; the light/dark command). */
export function getSchemeTheme(scheme: ThemeScheme): string {
  return pick[scheme];
}

function css(id: string, t: ResolvedTheme): string {
  const decls = [`color-scheme: ${t.scheme};`, ...resolvedThemeEntries(t).map(([k, v]) => `${k}: ${v};`)];
  return `html[data-theme-id="${id}"] {\n  ${decls.join("\n  ")}\n}\n`;
}

/** Puts the applied theme on `<html>` (see the module docs); remembers it unless `persist` is off. */
function apply(persist = true): void {
  if (typeof document === "undefined") return;
  const def = preview ?? findTheme(currentId) ?? BUILTIN_THEMES[0];
  const t = resolved(def);
  const id = preview ? PREVIEW_ID : def.id;
  const root = document.documentElement;
  let style = document.getElementById(STYLE_ID);
  if (!isBuiltinTheme(id)) {
    if (!style) {
      style = document.createElement("style");
      style.id = STYLE_ID;
      document.head.appendChild(style);
    }
    style.textContent = css(id, t);
  } else if (style) style.textContent = "";
  root.dataset.theme = t.scheme;
  root.dataset.themeId = id;
  root.dataset.themeRev = String(++rev);
  if (persist && !preview) {
    write(SCHEME_KEY, t.scheme);
    write(ID_KEY, def.id);
  }
  listeners.forEach((l) => l());
}

/** Applies theme `id` (a built-in or one of the user's), stops following the system, remembers it. */
export function setThemeId(id: string): void {
  const def = findTheme(id);
  if (!def) return;
  currentId = id;
  pick = { ...pick, [def.base]: id };
  write(PICK_KEY[def.base], id);
  if (mode !== "fixed") {
    mode = "fixed";
    write(MODE_KEY, mode);
  }
  apply();
}

/** Applies the theme used for `scheme` (the light/dark command, `setTheme`). */
export function setScheme(scheme: ThemeScheme): void {
  setThemeId(findTheme(pick[scheme])?.base === scheme ? pick[scheme] : SCHEME_DEFAULT_THEME[scheme]);
}

/** Follows the system's light/dark (`system`), with the theme picked for each, or not. */
export function setThemeMode(next: ThemeMode): void {
  mode = next;
  write(MODE_KEY, next);
  if (next === "system") {
    watchSystem();
    currentId = pick[systemScheme()];
  }
  apply();
}

/** Picks the theme used for `scheme` when following the system. */
export function setSchemeTheme(scheme: ThemeScheme, id: string): void {
  if (findTheme(id)?.base !== scheme) return;
  pick = { ...pick, [scheme]: id };
  write(PICK_KEY[scheme], id);
  if (mode === "system" && systemScheme() === scheme) currentId = id;
  apply();
}

/** Shows `def` on the whole app without choosing it (the theme editor), or stops (`null`). */
export function previewTheme(def: ThemeDefinition | null): void {
  preview = def;
  apply(false);
}

/** Adds or replaces (same id) one of the user's themes; re-applies it if it's on screen. */
export function saveCustomTheme(def: ThemeDefinition): void {
  if (isBuiltinTheme(def.id)) return;
  const at = custom.findIndex((d) => d.id === def.id);
  setCustom(at < 0 ? [...custom, def] : custom.map((d, i) => (i === at ? def : d)));
  apply(currentId === def.id);
}

/** Deletes one of the user's themes; if it was on screen, its scheme's built-in theme takes over. */
export function deleteCustomTheme(id: string): void {
  const def = custom.find((d) => d.id === id);
  if (!def) return;
  setCustom(custom.filter((d) => d.id !== id));
  for (const s of ["dark", "light"] as const) {
    if (pick[s] === id) {
      pick = { ...pick, [s]: SCHEME_DEFAULT_THEME[s] };
      write(PICK_KEY[s], pick[s]);
    }
  }
  if (currentId === id) currentId = SCHEME_DEFAULT_THEME[def.base];
  apply();
}

/** A fresh id for a theme of the user's. */
export function newCustomThemeId(): string {
  let id: string;
  do id = `custom-${Math.random().toString(36).slice(2, 10)}`;
  while (findTheme(id));
  return id;
}

let mediaWatched = false;

/**
 * Applies the remembered theme: the one following the system, else the remembered id when it
 * matches the remembered scheme (`eth-theme`, which tests and older versions write alone), else
 * the theme picked for that scheme. Also the docs page's theme.
 */
export function initThemeSelection(): void {
  if (typeof document === "undefined") return;
  setCustom(loadCustom(), false);
  for (const s of ["dark", "light"] as const) {
    const p = read(PICK_KEY[s]);
    if (p && findTheme(p)?.base === s) pick[s] = p;
  }
  mode = read(MODE_KEY) === "system" ? "system" : "fixed";
  const stored = read(SCHEME_KEY);
  const scheme: ThemeScheme = mode === "system" ? systemScheme() : stored === "light" ? "light" : "dark";
  const id = read(ID_KEY);
  currentId = mode === "fixed" && id && findTheme(id)?.base === scheme ? id : pick[scheme];
  apply(false);
  watchSystem();
}

/** Re-applies when the system switches light/dark while following it (watched once). */
function watchSystem(): void {
  if (mediaWatched || typeof matchMedia === "undefined") return;
  mediaWatched = true;
  matchMedia("(prefers-color-scheme: light)").addEventListener?.("change", () => {
    if (mode !== "system") return;
    currentId = pick[systemScheme()];
    apply();
  });
}

/** Calls `l` after every applied change. */
export function subscribeTheme(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}
