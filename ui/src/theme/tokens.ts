/**
 * Ethereal design tokens: THE single place to restyle the app.
 *
 * Edit values here. The dev server (`just dev-ui`) regenerates `tokens.css` automatically on
 * save (Vite plugin in tokens-plugin.ts); otherwise run `just gen-tokens`.
 * `tokens.test.ts` fails if `tokens.css` is out of date. Every value becomes a CSS custom
 * property named `--eth-<group>-<kebab-key>` (e.g. `colors.bgPanel` → `--eth-color-bg-panel`),
 * except the track palette (`--eth-track-<i>`) and component tokens (`--knob-track`, ...),
 * which keep their own names. See docs/DESIGN-SYSTEM.md.
 */

// ---------------------------------------------------------------------------------------
// Theme-dependent tokens (one set per theme; the key sets must match).
// ---------------------------------------------------------------------------------------

export const darkColors = {
  // Surfaces, darkest (inset wells) to lightest (popovers).
  bg: "#1e1e1e",
  bgInset: "#161616",
  bgPanel: "#2a2a2a",
  bgRaised: "#353535",
  bgHover: "#444444",
  bgActive: "#505050",
  bgOverlay: "#303030",
  backdrop: "rgba(0, 0, 0, 0.55)",
  // Borders.
  border: "#0f0f0f",
  borderLight: "#444444",
  borderStrong: "#5a5a5a",
  // Text.
  text: "#d8d8d8",
  textDim: "#8a8a8a",
  textDisabled: "#5c5c5c",
  textInverse: "#111111",
  // Accent (primary interactive color).
  accent: "#ffa94d",
  accentHover: "#ffbb70",
  accentDim: "#b36b1f",
  accentSubtle: "rgba(255, 169, 77, 0.18)",
  // Semantic.
  ok: "#6bd46b",
  warn: "#e6d34a",
  danger: "#ff4d4d",
  info: "#4d8cff",
  selection: "#4d8cff",
  focus: "#4d8cff",
  // DAW-specific.
  playhead: "#f5f5f5",
  record: "#ff4d4d",
  play: "#6bd46b",
  meterLow: "#4fd66a",
  meterMid: "#e6d34a",
  meterHigh: "#ff4d4d",
  knobTrack: "#4a4a4a",
} as const;

export type ColorKey = keyof typeof darkColors;
export type ColorTokens = Record<ColorKey, string>;

export const lightColors: ColorTokens = {
  bg: "#c9c9c9",
  bgInset: "#e8e8e8",
  bgPanel: "#d6d6d6",
  bgRaised: "#e2e2e2",
  bgHover: "#ececec",
  bgActive: "#f4f4f4",
  bgOverlay: "#efefef",
  backdrop: "rgba(0, 0, 0, 0.3)",
  border: "#9a9a9a",
  borderLight: "#b4b4b4",
  borderStrong: "#7a7a7a",
  text: "#1c1c1c",
  textDim: "#5a5a5a",
  textDisabled: "#9a9a9a",
  textInverse: "#ffffff",
  accent: "#e07b12",
  accentHover: "#f08d27",
  accentDim: "#f2b06b",
  accentSubtle: "rgba(224, 123, 18, 0.18)",
  ok: "#2f9e44",
  warn: "#b8930c",
  danger: "#d62c2c",
  info: "#2466d6",
  selection: "#2466d6",
  focus: "#2466d6",
  playhead: "#111111",
  record: "#d62c2c",
  play: "#2f9e44",
  meterLow: "#2fb34a",
  meterMid: "#d1b21c",
  meterHigh: "#e03131",
  knobTrack: "#a8a8a8",
};

export const darkShadows = {
  sm: "0 1px 2px rgba(0, 0, 0, 0.5)",
  md: "0 4px 12px rgba(0, 0, 0, 0.5)",
  lg: "0 12px 32px rgba(0, 0, 0, 0.6)",
  inset: "inset 0 1px 2px rgba(0, 0, 0, 0.5)",
} as const;

export type ShadowTokens = Record<keyof typeof darkShadows, string>;

export const lightShadows: ShadowTokens = {
  sm: "0 1px 2px rgba(0, 0, 0, 0.15)",
  md: "0 4px 12px rgba(0, 0, 0, 0.18)",
  lg: "0 12px 32px rgba(0, 0, 0, 0.22)",
  inset: "inset 0 1px 2px rgba(0, 0, 0, 0.15)",
};

export interface ThemeTokens {
  colorScheme: "dark" | "light";
  color: ColorTokens;
  shadow: ShadowTokens;
}

export const themes = {
  dark: { colorScheme: "dark", color: darkColors, shadow: darkShadows },
  light: { colorScheme: "light", color: lightColors, shadow: lightShadows },
} satisfies Record<string, ThemeTokens>;

export type ThemeName = keyof typeof themes;
export const THEME_NAMES = Object.keys(themes) as ThemeName[];
export const DEFAULT_THEME: ThemeName = "dark";

// ---------------------------------------------------------------------------------------
// Theme-independent tokens.
// ---------------------------------------------------------------------------------------

export const space = {
  "0": "0",
  xs: "2px",
  sm: "4px",
  md: "8px",
  lg: "12px",
  xl: "16px",
  "2xl": "24px",
  "3xl": "32px",
} as const;

export const radius = {
  none: "0",
  sm: "2px",
  md: "4px",
  lg: "6px",
  xl: "10px",
  pill: "999px",
  round: "50%",
} as const;

export const border = {
  width: "1px",
  widthStrong: "2px",
} as const;

/** Font sizes (`--eth-fs-*`). */
export const fontSize = {
  xs: "10px",
  sm: "11px",
  md: "12px",
  lg: "14px",
  xl: "18px",
  "2xl": "24px",
} as const;

/** Font weights (`--eth-fw-*`). */
export const fontWeight = {
  regular: "400",
  medium: "500",
  bold: "600",
} as const;

/** Line heights (`--eth-lh-*`). */
export const lineHeight = {
  tight: "1.1",
  normal: "1.35",
} as const;

/** Letter spacing (`--eth-tracking-*`). */
export const letterSpacing = {
  normal: "0",
  caps: "0.04em",
} as const;

export const font = {
  ui: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
  mono: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
} as const;

export const focus = {
  ringWidth: "1px",
  ringOffset: "1px",
} as const;

export const duration = {
  instant: "0ms",
  fast: "80ms",
  normal: "150ms",
  slow: "300ms",
  tooltipDelay: "500ms",
} as const;

export const ease = {
  standard: "cubic-bezier(0.2, 0, 0, 1)",
  out: "cubic-bezier(0, 0, 0.2, 1)",
  in: "cubic-bezier(0.4, 0, 1, 1)",
} as const;

export const opacity = {
  disabled: "0.4",
  muted: "0.6",
} as const;

export const zIndex = {
  popover: "100",
  tooltip: "200",
  dialog: "300",
} as const;

/** Component and layout dimensions (`--eth-size-*`). */
export const size = {
  controlSm: "18px",
  controlMd: "22px",
  controlLg: "28px",
  iconSm: "12px",
  iconMd: "14px",
  iconLg: "18px",
  knobSm: "24px",
  knobMd: "32px",
  knobLg: "44px",
  knobStroke: "3px",
  knobPointer: "2px",
  faderWidth: "24px",
  faderTrackWidth: "4px",
  faderThumbHeight: "10px",
  faderHeight: "120px",
  meterHeight: "120px",
  meterChannelWidth: "5px",
  meterGap: "1px",
  meterClipHeight: "3px",
  toggleWidthSm: "20px",
  toggleHeightSm: "11px",
  toggleWidth: "26px",
  toggleHeight: "14px",
  toggleWidthLg: "34px",
  toggleHeightLg: "18px",
  rowHeight: "22px",
  rowHeightSm: "18px",
  rowHeightLg: "28px",
  panelHeaderHeight: "22px",
  topBarHeight: "36px",
  sidebarWidth: "220px",
  detailHeight: "280px",
  detailCollapsedHeight: "24px",
  popoverMinWidth: "140px",
  dialogWidth: "420px",
  tooltipMaxWidth: "240px",
} as const;

/** Meter gradient stop positions (`--eth-meter-*`); colors are `meterLow/Mid/High`. */
export const meter = {
  stopMid: "70%",
  stopHigh: "85%",
} as const;

/**
 * Knob drawing geometry (`--eth-knob-geometry-*`), in a 100×100 viewBox with 0° = up.
 * Read by Knob.tsx (SVG paths can't use CSS vars). Strokes are CSS tokens (`--knob-stroke`).
 */
export const knobGeometry = {
  /** Angle of the minimum value, degrees. */
  startAngle: "-135",
  /** Total sweep from minimum to maximum, degrees. */
  sweep: "270",
  /** Arc radius (viewBox units, max 50). */
  radius: "42",
  /** Pointer end distance from the center (viewBox units). */
  pointerLength: "30",
  /** Pointer start distance from the center (0 = from the center). */
  pointerInset: "0",
} as const;

/** Ableton-ish clip/track color palette (`--eth-track-<i>`). Index with `trackColor(i)`. */
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

// ---------------------------------------------------------------------------------------
// Component tokens: per-component knobs for shape/color, defaulting to global tokens.
// Override any of them here (all themes), in a `[data-theme="..."]` block, or on a container
// (`.my-panel { --knob-value: var(--eth-color-danger) }`). Values may reference global tokens.
// ---------------------------------------------------------------------------------------

const v = (name: string): string => `var(--eth-${name})`;

export const componentTokens = {
  button: {
    "--button-height": v("size-control-md"),
    "--button-padding-x": v("space-md"),
    "--button-font-size": v("fs-sm"),
    "--button-height-sm": v("size-control-sm"),
    "--button-padding-x-sm": v("space-sm"),
    "--button-font-size-sm": v("fs-xs"),
    "--button-height-lg": v("size-control-lg"),
    "--button-padding-x-lg": v("space-lg"),
    "--button-font-size-lg": v("fs-md"),
    "--button-radius": v("radius-sm"),
    "--button-border": v("color-border"),
    "--button-bg": v("color-bg-raised"),
    "--button-bg-hover": v("color-bg-hover"),
    "--button-fg": v("color-text"),
    "--button-accent-bg": v("color-accent"),
    "--button-accent-bg-hover": v("color-accent-hover"),
    "--button-accent-fg": v("color-text-inverse"),
    "--button-danger-bg": v("color-danger"),
    "--button-danger-fg": v("color-text-inverse"),
    "--button-ghost-bg-hover": v("color-bg-hover"),
  },
  knob: {
    "--knob-size": v("size-knob-md"),
    "--knob-size-sm": v("size-knob-sm"),
    "--knob-size-lg": v("size-knob-lg"),
    "--knob-track": v("color-knob-track"),
    "--knob-value": v("color-accent"),
    "--knob-pointer": v("color-text"),
    "--knob-stroke": v("size-knob-stroke"),
    "--knob-pointer-width": v("size-knob-pointer"),
    "--knob-pointer-linecap": "round",
    "--knob-linecap": "butt",
    "--knob-body": "transparent",
    "--knob-body-border": "transparent",
    "--knob-label-fg": v("color-text-dim"),
  },
  fader: {
    "--fader-width": v("size-fader-width"),
    "--fader-height": v("size-fader-height"),
    "--fader-track-width": v("size-fader-track-width"),
    "--fader-track-bg": v("color-bg-inset"),
    "--fader-track-radius": v("radius-sm"),
    "--fader-fill": v("color-accent-dim"),
    "--fader-thumb-height": v("size-fader-thumb-height"),
    "--fader-thumb-bg": v("color-text-dim"),
    "--fader-thumb-border": v("color-border"),
    "--fader-thumb-radius": v("radius-sm"),
  },
  meter: {
    "--meter-height": v("size-meter-height"),
    "--meter-channel-width": v("size-meter-channel-width"),
    "--meter-gap": v("size-meter-gap"),
    "--meter-bg": v("color-bg-inset"),
    "--meter-radius": v("radius-none"),
    "--meter-low": v("color-meter-low"),
    "--meter-mid": v("color-meter-mid"),
    "--meter-high": v("color-meter-high"),
    "--meter-stop-mid": v("meter-stop-mid"),
    "--meter-stop-high": v("meter-stop-high"),
    "--meter-clip-height": v("size-meter-clip-height"),
  },
  panel: {
    "--panel-bg": v("color-bg-panel"),
    "--panel-border": v("color-border"),
    "--panel-radius": v("radius-md"),
    "--panel-header-height": v("size-panel-header-height"),
    "--panel-header-bg": v("color-bg-raised"),
    "--panel-header-fg": v("color-text-dim"),
    "--panel-header-font-size": v("fs-sm"),
    "--panel-header-transform": "uppercase",
  },
  tabs: {
    "--tab-height": v("size-control-md"),
    "--tab-padding-x": v("space-md"),
    "--tab-font-size": v("fs-sm"),
    "--tab-height-sm": v("size-control-sm"),
    "--tab-padding-x-sm": v("space-sm"),
    "--tab-font-size-sm": v("fs-xs"),
    "--tab-height-lg": v("size-control-lg"),
    "--tab-padding-x-lg": v("space-lg"),
    "--tab-font-size-lg": v("fs-md"),
    "--tab-fg": v("color-text-dim"),
    "--tab-fg-active": v("color-text-inverse"),
    "--tab-bg-hover": v("color-bg-hover"),
    "--tab-bg-active": v("color-accent"),
    "--tab-radius": v("radius-sm"),
  },
  toggle: {
    "--toggle-width": v("size-toggle-width"),
    "--toggle-height": v("size-toggle-height"),
    "--toggle-width-sm": v("size-toggle-width-sm"),
    "--toggle-height-sm": v("size-toggle-height-sm"),
    "--toggle-width-lg": v("size-toggle-width-lg"),
    "--toggle-height-lg": v("size-toggle-height-lg"),
    "--toggle-font-size": v("fs-sm"),
    "--toggle-font-size-sm": v("fs-xs"),
    "--toggle-font-size-lg": v("fs-md"),
    "--toggle-track-off": v("color-bg-inset"),
    "--toggle-track-on": v("color-accent"),
    "--toggle-thumb": v("color-text"),
    "--toggle-radius": v("radius-pill"),
  },
  input: {
    "--input-height": v("size-control-md"),
    "--input-padding-x": v("space-sm"),
    "--input-font-size": v("fs-sm"),
    "--input-bg": v("color-bg-inset"),
    "--input-fg": v("color-text"),
    "--input-border": v("color-border"),
    "--input-border-focus": v("color-focus"),
    "--input-radius": v("radius-sm"),
    "--input-height-sm": v("size-control-sm"),
    "--input-padding-x-sm": v("space-sm"),
    "--input-font-size-sm": v("fs-xs"),
    "--input-height-lg": v("size-control-lg"),
    "--input-padding-x-lg": v("space-md"),
    "--input-font-size-lg": v("fs-md"),
  },
  popover: {
    "--popover-bg": v("color-bg-overlay"),
    "--popover-border": v("color-border-light"),
    "--popover-radius": v("radius-md"),
    "--popover-shadow": v("shadow-md"),
    "--menu-item-height": v("size-row-height"),
    "--menu-item-bg-hover": v("color-accent-subtle"),
  },
  dialog: {
    "--dialog-width": v("size-dialog-width"),
    "--dialog-bg": v("color-bg-panel"),
    "--dialog-border": v("color-border-light"),
    "--dialog-radius": v("radius-lg"),
    "--dialog-shadow": v("shadow-lg"),
    "--dialog-backdrop": v("color-backdrop"),
  },
  tooltip: {
    "--tooltip-bg": v("color-bg-overlay"),
    "--tooltip-fg": v("color-text"),
    "--tooltip-border": v("color-border-light"),
    "--tooltip-radius": v("radius-sm"),
    "--tooltip-delay": v("duration-tooltip-delay"),
  },
  badge: {
    "--badge-radius": v("radius-pill"),
    "--badge-bg": v("color-bg-raised"),
    "--badge-fg": v("color-text"),
    "--badge-font-size": v("fs-xs"),
  },
} as const satisfies Record<string, Record<`--${string}`, string>>;

// ---------------------------------------------------------------------------------------
// Groups → CSS custom property names.
// ---------------------------------------------------------------------------------------

/** Theme-independent groups, keyed by the CSS name segment (`--eth-<segment>-<key>`). */
export const sharedGroups: Record<string, Record<string, string>> = {
  space,
  radius,
  border,
  fs: fontSize,
  fw: fontWeight,
  lh: lineHeight,
  tracking: letterSpacing,
  font,
  focus,
  duration,
  ease,
  opacity,
  z: zIndex,
  size,
  meter,
  "knob-geometry": knobGeometry,
};

function kebab(s: string): string {
  return s.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);
}

/** Name of a token's CSS custom property, e.g. `tokenName("color", "bgPanel")` → `--eth-color-bg-panel`. */
export function tokenName(group: string, key: string): string {
  return `--eth-${group}-${kebab(key)}`;
}

/** Theme-independent tokens as `[customProperty, value]` pairs (incl. `--eth-track-<i>`). */
export function sharedEntries(): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  for (const [group, tokens] of Object.entries(sharedGroups)) {
    for (const [key, value] of Object.entries(tokens)) out.push([tokenName(group, key), value]);
  }
  TRACK_COLORS.forEach((c, i) => out.push([`--eth-track-${i}`, c]));
  return out;
}

/** One theme's tokens as `[customProperty, value]` pairs. */
export function themeOnlyEntries(theme: ThemeName): Array<[string, string]> {
  const t: ThemeTokens = themes[theme];
  const out: Array<[string, string]> = [];
  for (const [key, value] of Object.entries(t.color)) out.push([tokenName("color", key), value]);
  for (const [key, value] of Object.entries(t.shadow)) out.push([tokenName("shadow", key), value]);
  return out;
}

/** Component tokens as `[customProperty, value]` pairs. */
export function componentEntries(): Array<[string, string]> {
  return Object.values(componentTokens).flatMap((g) => Object.entries(g) as Array<[string, string]>);
}

/** Every token that applies under `theme`, as `[customProperty, value]` pairs. */
export function themeEntries(theme: ThemeName = DEFAULT_THEME): Array<[string, string]> {
  return [...sharedEntries(), ...themeOnlyEntries(theme), ...componentEntries()];
}
