// eth-allow-case-file: theme file format and token keys.
/**
 * The theme model: a theme is a definition (a name, a base scheme and some colors), the rest of
 * its color tokens derived from its main colors. Built-in themes (`builtinThemes.ts`) and the
 * user's own (Settings > Appearance) are both definitions; a theme file is one, as JSON
 * (docs/THEMES.md documents the format).
 *
 * Resolving a definition: the base scheme's built-in theme gives any main color the definition
 * leaves out, `deriveColors` computes every other token from the main colors, then the
 * definition's own values for any token win.
 */
import { adjustHsl, contrast, isColor, mix, withAlpha } from "./color";
import {
  darkColors,
  darkShadows,
  ink,
  lightColors,
  lightShadows,
  tokenName,
  type ColorKey,
  type ColorTokens,
  type ShadowTokens,
  type ThemeName,
} from "./tokens";

/** A theme's base: light or dark (CSS `color-scheme`, shadows, glow, light-only style rules). */
export type ThemeScheme = ThemeName;

/** The `format` field of a theme file. */
export const THEME_FORMAT = "ethereal-theme";
/** The theme file version this build writes, and the newest it reads. */
export const THEME_VERSION = 1;

/** Every color token, in `tokens.ts` order. */
export const COLOR_KEYS = Object.keys(darkColors) as ColorKey[];

/** The main colors: what the theme editor shows; every other color is derived from them. */
export const CORE_KEYS = ["bg", "bgPanel", "bgRaised", "text", "textDim", "accent", "ok", "warn", "danger", "info"] as const satisfies readonly ColorKey[];
export type CoreKey = (typeof CORE_KEYS)[number];

export interface ThemeDefinition {
  /** Unique: a built-in's name (`dark`), or `custom-…` for the user's themes. */
  id: string;
  /** What the theme picker shows (built-ins show a translated name instead). */
  name: string;
  base: ThemeScheme;
  /** Main colors and any token overrides; what's missing is taken from the base or derived. */
  colors: Partial<ColorTokens>;
}

/** A definition with every token resolved. */
export interface ResolvedTheme {
  id: string;
  name: string;
  scheme: ThemeScheme;
  color: ColorTokens;
  shadow: ShadowTokens;
}

const BASE_COLORS: Record<ThemeScheme, ColorTokens> = { dark: darkColors, light: lightColors };
const BASE_SHADOWS: Record<ThemeScheme, ShadowTokens> = { dark: darkShadows, light: lightShadows };

/** Of `candidates`, the one that contrasts most with `bg`. */
function bestOn(bg: string, candidates: string[]): string {
  return candidates.reduce((best, c) => (contrast(c, bg) > contrast(best, bg) ? c : best));
}

/**
 * Every color token from the main colors (see docs/THEMES.md "derived tokens"). Fixed: the
 * piano keyboard's keys and the loop seams, taken from the base scheme's built-in theme.
 */
export function deriveColors(core: Record<CoreKey, string>, scheme: ThemeScheme): ColorTokens {
  const { bg, bgPanel, bgRaised, text, textDim, accent, ok, warn, danger, info } = core;
  const dark = scheme === "dark";
  const base = BASE_COLORS[scheme];
  const white = "#ffffff";
  const black = "#000000";
  return {
    bg,
    bgInset: dark ? mix(bg, 0.78, black) : mix(bg, 0.96, text),
    bgPanel,
    bgRaised,
    bgCard: dark ? mix(text, 0.04, bgRaised) : mix(white, 0.6, bgRaised),
    bgHover: mix(text, 0.05, bgRaised),
    bgActive: mix(text, 0.1, bgRaised),
    bgOverlay: withAlpha(bgRaised, dark ? 0.96 : 0.97),
    backdrop: dark ? withAlpha(mix(bg, 0.5, black), 0.5) : withAlpha(text, 0.22),
    border: dark ? mix(bg, 0.65, black) : mix(text, 0.14, bgPanel),
    borderLight: dark ? mix(text, 0.055, bgRaised) : mix(text, 0.08, bgPanel),
    borderStrong: dark ? mix(text, 0.15, bgRaised) : mix(text, 0.25, bgPanel),
    borderCard: mix(text, 0.08, dark ? mix(text, 0.04, bgRaised) : white),
    text,
    textDim,
    textDisabled: mix(text, 0.33, bg),
    textInverse: bestOn(accent, [ink.dark, white, mix(bg, 0.6, black)]),
    accent,
    accentHover: mix(white, dark ? 0.15 : 0.1, accent),
    accentDim: dark ? mix(accent, 0.64, bg) : mix(accent, 0.38, bgRaised),
    accentSubtle: withAlpha(accent, dark ? 0.14 : 0.12),
    accentGlow: adjustHsl(accent, 0.28, dark ? -0.07 : -0.05),
    ok,
    warn,
    danger,
    info,
    selection: accent,
    focus: accent,
    playhead: dark ? mix(white, 0.3, text) : text,
    playheadPlaying: dark ? accent : text,
    playheadGlow: dark ? withAlpha(accent, 0.35) : "transparent",
    record: danger,
    play: ok,
    meterLow: ok,
    meterMid: warn,
    meterHigh: danger,
    knobTrack: dark ? mix(text, 0.055, bgRaised) : mix(text, 0.14, bgPanel),
    outsideShade: dark ? "rgba(0, 0, 0, 0.35)" : withAlpha(text, 0.08),
    clipSeam: base.clipSeam,
    warpWave: withAlpha(info, 0.85),
    warpWaveDim: withAlpha(info, 0.3),
    pianoWhiteKey: base.pianoWhiteKey,
    pianoWhiteKeyBorder: base.pianoWhiteKeyBorder,
    pianoBlackKey: base.pianoBlackKey,
    pianoKeyLabel: base.pianoKeyLabel,
    pianoScaleRow: mix(accent, 0.06, bgPanel),
    pianoScaleRootRow: mix(accent, 0.14, bgPanel),
    pianoScaleRootLine: withAlpha(accent, 0.35),
    pianoScaleKeyMark: withAlpha(accent, 0.55),
  };
}

/** The main colors of `def`: its own, else its base scheme's built-in ones. */
export function coreColors(def: Pick<ThemeDefinition, "base" | "colors">): Record<CoreKey, string> {
  const base = BASE_COLORS[def.base];
  return Object.fromEntries(CORE_KEYS.map((k) => [k, def.colors[k] ?? base[k]])) as Record<CoreKey, string>;
}

/** Every token of `def` (see the module docs). */
export function resolveColors(def: Pick<ThemeDefinition, "base" | "colors">): ColorTokens {
  const out = deriveColors(coreColors(def), def.base);
  for (const k of COLOR_KEYS) {
    const v = def.colors[k];
    if (v !== undefined && isColor(v)) out[k] = v;
  }
  return out;
}

export function resolveTheme(def: ThemeDefinition): ResolvedTheme {
  return { id: def.id, name: def.name, scheme: def.base, color: resolveColors(def), shadow: BASE_SHADOWS[def.base] };
}

/** A resolved theme's tokens as `[customProperty, value]` pairs (what `themeOnlyEntries` gives a scheme). */
export function resolvedThemeEntries(t: ResolvedTheme): Array<[string, string]> {
  return [
    ...COLOR_KEYS.map((k): [string, string] => [tokenName("color", k), t.color[k]]),
    ...Object.entries(t.shadow).map(([k, v]): [string, string] => [tokenName("shadow", k), v]),
  ];
}

// ---- Contrast -------------------------------------------------------------------------

/** One contrast requirement: `fg` on `bg` at least `min` (WCAG AA: 4.5 text, 3 controls). */
export interface ContrastCheck {
  fg: ColorKey;
  bg: ColorKey;
  min: number;
}

const surfaces: ColorKey[] = ["bg", "bgInset", "bgPanel", "bgRaised", "bgCard", "bgHover", "bgActive"];

/**
 * What every theme must pass (checked over the built-ins by a test, and shown live in the
 * theme editor): text and dim text on every surface they sit on (AA text, 4.5:1), text on the
 * accent and danger buttons, and the accent, focus ring and status colors as controls (3:1).
 */
export const CONTRAST_CHECKS: ContrastCheck[] = [
  ...surfaces.map((bg) => ({ fg: "text" as const, bg, min: 4.5 })),
  ...(["bg", "bgPanel", "bgRaised", "bgCard"] as const).map((bg) => ({ fg: "textDim" as const, bg, min: 4.5 })),
  { fg: "textInverse", bg: "accent", min: 4.5 },
  ...(["bg", "bgPanel", "bgRaised"] as const).flatMap((bg) =>
    (["accent", "focus", "ok", "warn", "danger", "info", "playhead"] as const).map((fg) => ({ fg, bg, min: 3 })),
  ),
];

export interface ContrastFailure extends ContrastCheck {
  ratio: number;
}

/** The checks `colors` fails. Surfaces are composited over `bg` (a translucent overlay). */
export function contrastFailures(colors: ColorTokens): ContrastFailure[] {
  const out: ContrastFailure[] = [];
  for (const c of CONTRAST_CHECKS) {
    const ratio = contrast(colors[c.fg], colors[c.bg]);
    if (ratio + 1e-9 < c.min) out.push({ ...c, ratio });
  }
  return out;
}

// ---- Theme files ----------------------------------------------------------------------

/** A theme file (`.json`): what export writes and import reads. */
export interface ThemeFile {
  format: typeof THEME_FORMAT;
  version: number;
  name: string;
  base: ThemeScheme;
  colors: Partial<ColorTokens>;
}

/** Longest theme name kept (longer ones are cut). */
export const THEME_NAME_MAX = 60;

/** The JSON text of `def` as a theme file (keys in token order). */
export function serializeTheme(def: Pick<ThemeDefinition, "name" | "base" | "colors">): string {
  const colors: Partial<ColorTokens> = {};
  for (const k of COLOR_KEYS) if (def.colors[k] !== undefined) colors[k] = def.colors[k];
  const file: ThemeFile = { format: THEME_FORMAT, version: THEME_VERSION, name: def.name, base: def.base, colors };
  return `${JSON.stringify(file, null, 2)}\n`;
}

/** Why a theme file was refused. */
export type ThemeFileError =
  { code: "not_json" } | { code: "not_theme" } | { code: "newer_version"; version: number } | { code: "bad_version" } | { code: "bad_base" };

/** What was repaired in an accepted file. */
export type ThemeFileWarning = { code: "unknown_tokens"; tokens: string[] } | { code: "invalid_colors"; tokens: string[] } | { code: "no_name" };

export type ParsedThemeFile = { ok: true; theme: Omit<ThemeDefinition, "id">; warnings: ThemeFileWarning[] } | { ok: false; error: ThemeFileError };

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/**
 * Reads a theme file. Refuses what isn't one, a newer version and a missing base; repairs the
 * rest: unknown tokens and values that aren't colors are dropped (derived instead), a missing
 * name gets a placeholder (`""`, the caller names it), and each repair is reported.
 */
export function parseThemeFile(text: string): ParsedThemeFile {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return { ok: false, error: { code: "not_json" } };
  }
  if (!isRecord(data) || data.format !== THEME_FORMAT) return { ok: false, error: { code: "not_theme" } };
  const version = data.version;
  if (typeof version !== "number" || !Number.isInteger(version) || version < 1) return { ok: false, error: { code: "bad_version" } };
  if (version > THEME_VERSION) return { ok: false, error: { code: "newer_version", version } };
  if (data.base !== "dark" && data.base !== "light") return { ok: false, error: { code: "bad_base" } };
  const warnings: ThemeFileWarning[] = [];
  const rawName = typeof data.name === "string" ? data.name.trim().slice(0, THEME_NAME_MAX) : "";
  if (!rawName) warnings.push({ code: "no_name" });
  const colors: Partial<ColorTokens> = {};
  const unknown: string[] = [];
  const invalid: string[] = [];
  const known = new Set<string>(COLOR_KEYS);
  for (const [k, v] of Object.entries(isRecord(data.colors) ? data.colors : {})) {
    if (!known.has(k)) unknown.push(k);
    else if (!isColor(v)) invalid.push(k);
    else colors[k as ColorKey] = v.trim();
  }
  if (unknown.length) warnings.push({ code: "unknown_tokens", tokens: unknown });
  if (invalid.length) warnings.push({ code: "invalid_colors", tokens: invalid });
  return { ok: true, theme: { name: rawName, base: data.base, colors }, warnings };
}

/** Whether `v` is a usable stored definition (local storage can hold anything). */
export function isThemeDefinition(v: unknown): v is ThemeDefinition {
  if (!isRecord(v) || typeof v.id !== "string" || typeof v.name !== "string") return false;
  if (v.base !== "dark" && v.base !== "light") return false;
  return isRecord(v.colors) && Object.entries(v.colors).every(([k, c]) => (COLOR_KEYS as string[]).includes(k) && isColor(c));
}
