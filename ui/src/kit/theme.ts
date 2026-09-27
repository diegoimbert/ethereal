/**
 * Design tokens. Single source of truth for colors/spacing/typography.
 *
 * Tokens are exposed to CSS as custom properties (`--eth-*`), declared in `theme.css`
 * (kept in sync with this file by `theme.test.ts`). Use `cssVar("bg")` in inline styles,
 * and the raw values (e.g. `TRACK_COLORS`) where CSS vars don't reach (canvas drawing).
 */

export const colors = {
  bg: "#1e1e1e",
  bgPanel: "#2a2a2a",
  bgRaised: "#353535",
  bgInset: "#161616",
  border: "#0f0f0f",
  borderLight: "#444444",
  text: "#d8d8d8",
  textDim: "#8a8a8a",
  textInverse: "#111111",
  accent: "#ffa94d",
  accentDim: "#b36b1f",
  selection: "#4d8cff",
  playhead: "#f5f5f5",
  record: "#ff4d4d",
  play: "#6bd46b",
  meterLow: "#4fd66a",
  meterMid: "#e6d34a",
  meterHigh: "#ff4d4d",
  knobTrack: "#4a4a4a",
} as const;

export const space = {
  xs: "2px",
  sm: "4px",
  md: "8px",
  lg: "12px",
  xl: "16px",
} as const;

export const radius = {
  sm: "2px",
  md: "4px",
  lg: "6px",
} as const;

export const fontSize = {
  xs: "10px",
  sm: "11px",
  md: "12px",
  lg: "14px",
} as const;

export const font = {
  ui: "'Inter Variable', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
  mono: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
} as const;

/** Ableton-ish clip/track color palette. Index with `trackColor(i)`. */
export const TRACK_COLORS = [
  "#ff94a6",
  "#ffa529",
  "#cc9927",
  "#f7f47c",
  "#bffb00",
  "#1aff2f",
  "#25ffa8",
  "#5cffe8",
  "#8bc5ff",
  "#5480e4",
  "#92a7ff",
  "#d86ce4",
  "#e553a0",
  "#ffffff",
  "#aa8c75",
  "#6e8c73",
] as const;

export function trackColor(index: number): string {
  const n = TRACK_COLORS.length;
  return TRACK_COLORS[((index % n) + n) % n] as string;
}

type TokenGroup = Record<string, string>;

const groups: Record<string, TokenGroup> = {
  color: colors,
  space,
  radius,
  fs: fontSize,
  font,
};

function kebab(s: string): string {
  return s.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);
}

/** Name of a token's CSS custom property, e.g. `tokenName("color", "bgPanel")` → `--eth-color-bg-panel`. */
export function tokenName(group: string, key: string): string {
  return `--eth-${group}-${kebab(key)}`;
}

/** `var(--eth-color-<key>)` for use in inline styles. */
export function cssVar(key: keyof typeof colors): string {
  return `var(${tokenName("color", key)})`;
}

/** All tokens as `[customProperty, value]` pairs (including `--eth-track-<i>`). */
export function themeEntries(): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  for (const [group, tokens] of Object.entries(groups)) {
    for (const [key, value] of Object.entries(tokens)) out.push([tokenName(group, key), value]);
  }
  TRACK_COLORS.forEach((c, i) => out.push([`--eth-track-${i}`, c]));
  return out;
}
