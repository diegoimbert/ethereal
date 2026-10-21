// eth-allow-case-file: font stacks and token values.
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
  // Surfaces: soft charcoal with a faint cool tint, darkest (inset wells) to lightest.
  bg: "#16181e",
  bgInset: "#111318",
  bgPanel: "#1c1f26",
  bgRaised: "#242830",
  // Cards on a raised pane (device cards in the inspector): a step lighter, so they stand out.
  bgCard: "#2a2f39",
  bgHover: "#2c313b",
  bgActive: "#353b47",
  bgOverlay: "rgba(36, 40, 48, 0.96)",
  backdrop: "rgba(8, 9, 12, 0.5)",
  // Borders.
  border: "#0e1014",
  borderLight: "#2e333e",
  borderStrong: "#3f4553",
  // The edge of a card (`bgCard`).
  borderCard: "#3a404b",
  // Text: slightly softened, never pure white.
  text: "#d8dbe3",
  textDim: "#8b90a0",
  textDisabled: "#555a67",
  textInverse: "#14161b",
  // Accent (primary interactive color): warm orchid (pink-purple).
  accent: "#d48fd0",
  accentHover: "#dfa3db",
  accentDim: "#8f5b8c",
  accentSubtle: "rgba(212, 143, 208, 0.14)",
  /** The accent, saturated: glows (selected notes), where the accent alone reads washed out. */
  accentGlow: "#e35ce0",
  // Semantic: muted.
  ok: "#7fcf9a",
  warn: "#e3c46f",
  danger: "#e58080",
  info: "#86b4e8",
  selection: "#d48fd0",
  focus: "#d48fd0",
  // DAW-specific.
  playhead: "#e6e8ef",
  // The playhead while playing: the accent with a soft halo (dark theme only).
  playheadPlaying: "#d48fd0",
  playheadGlow: "rgba(212, 143, 208, 0.35)",
  record: "#e58080",
  play: "#7fcf9a",
  meterLow: "#6fcfa4",
  meterMid: "#e3c46f",
  meterHigh: "#e58080",
  knobTrack: "#2e333e",
  // Editors.
  /** Shade over the part of an editor outside the clip (piano roll). */
  outsideShade: "rgb(0 0 0 / 0.35)",
  /** Loop seams drawn over clip previews (arrangement canvas). */
  clipSeam: "rgba(0, 0, 0, 0.25)",
  /** Waveform in the warp editor: the played range, and the rest of the file. */
  warpWave: "rgba(120, 200, 255, 0.85)",
  warpWaveDim: "rgba(120, 200, 255, 0.3)",
  /** Piano roll keyboard (black keys stay dark in every theme). */
  pianoWhiteKey: "#d8d8d8",
  pianoWhiteKeyBorder: "#aaa",
  pianoBlackKey: "#222",
  pianoKeyLabel: "#333",
  /** Piano-roll scale highlight: in-scale and root rows, the root row's line, key marks. */
  pianoScaleRow: "#25222c",
  pianoScaleRootRow: "#362c3c",
  pianoScaleRootLine: "rgba(212, 143, 208, 0.35)",
  pianoScaleKeyMark: "rgba(212, 143, 208, 0.55)",
} as const;

export type ColorKey = keyof typeof darkColors;
export type ColorTokens = Record<ColorKey, string>;

export const lightColors: ColorTokens = {
  bg: "#eceef2",
  bgInset: "#e4e6ec",
  bgPanel: "#f5f6f8",
  bgRaised: "#fbfbfc",
  bgCard: "#ffffff",
  bgHover: "#eef0f4",
  bgActive: "#e3e6ec",
  bgOverlay: "rgba(251, 251, 252, 0.97)",
  backdrop: "rgba(24, 27, 36, 0.22)",
  border: "#d3d7df",
  borderLight: "#e0e3e9",
  borderStrong: "#b6bcc8",
  borderCard: "#dde0e7",
  text: "#23262f",
  textDim: "#666c7b",
  textDisabled: "#a8adb9",
  textInverse: "#ffffff",
  accent: "#b25ca8",
  accentHover: "#be6fb5",
  accentDim: "#e6c1e1",
  accentSubtle: "rgba(178, 92, 168, 0.12)",
  accentGlow: "#c93dc5",
  ok: "#3f9a63",
  warn: "#b08a2a",
  danger: "#cc5a5a",
  info: "#4f86c6",
  selection: "#b25ca8",
  focus: "#b25ca8",
  playhead: "#23262f",
  // Light theme: the playing playhead looks like the parked one, no halo.
  playheadPlaying: "#23262f",
  playheadGlow: "transparent",
  record: "#cc5a5a",
  play: "#3f9a63",
  meterLow: "#3fa57a",
  meterMid: "#c9a23a",
  meterHigh: "#cc5a5a",
  knobTrack: "#d7dbe3",
  outsideShade: "rgba(24, 27, 36, 0.08)",
  clipSeam: "rgba(0, 0, 0, 0.25)",
  warpWave: "rgba(47, 124, 190, 0.85)",
  warpWaveDim: "rgba(47, 124, 190, 0.3)",
  pianoWhiteKey: "#fdfdfe",
  pianoWhiteKeyBorder: "#c3c8d3",
  pianoBlackKey: "#222",
  pianoKeyLabel: "#333",
  pianoScaleRow: "#e7e2ec",
  pianoScaleRootRow: "#e2d5e5",
  pianoScaleRootLine: "rgba(178, 92, 168, 0.35)",
  pianoScaleKeyMark: "rgba(178, 92, 168, 0.55)",
};

export const darkShadows = {
  sm: "0 1px 2px rgba(5, 6, 9, 0.4)",
  md: "0 6px 18px rgba(5, 6, 9, 0.35), 0 0 0 1px rgba(255, 255, 255, 0.03)",
  lg: "0 18px 48px rgba(5, 6, 9, 0.45), 0 0 0 1px rgba(255, 255, 255, 0.04)",
  inset: "inset 0 1px 2px rgba(5, 6, 9, 0.45)",
} as const;

export type ShadowTokens = Record<keyof typeof darkShadows, string>;

export const lightShadows: ShadowTokens = {
  sm: "0 1px 2px rgba(22, 25, 37, 0.08)",
  md: "0 6px 18px rgba(22, 25, 37, 0.1), 0 0 0 1px rgba(22, 25, 37, 0.04)",
  lg: "0 18px 48px rgba(22, 25, 37, 0.16), 0 0 0 1px rgba(22, 25, 37, 0.05)",
  inset: "inset 0 1px 2px rgba(22, 25, 37, 0.08)",
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
  /** One pixel: hairline insets and nudges. */
  hair: "1px",
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
  "2xs": "1px",
  xs: "2px",
  /** Plugin browser search field and badges. */
  compact: "3px",
  sm: "4px",
  md: "6px",
  lg: "8px",
  xl: "12px",
  pill: "999px",
  round: "50%",
} as const;

export const border = {
  width: "1px",
  widthStrong: "2px",
} as const;

/** Font sizes (`--eth-fs-*`). */
export const fontSize = {
  /** Piano roll key labels. */
  "2xs": "10.5px",
  xs: "12px",
  sm: "13px",
  md: "14px",
  lg: "16px",
  xl: "20px",
  "2xl": "26px",
  /** The welcome screen's "Ethereal" wordmark. */
  display: "44px",
  /** Text fields on touch screens: iOS zooms into a focused field under 16 px. */
  touchInput: "16px",
} as const;

/** Font weights (`--eth-fw-*`). */
export const fontWeight = {
  /** Body text. */
  text: "400",
  regular: "400",
  medium: "500",
  bold: "600",
} as const;

/** Line heights (`--eth-lh-*`). */
export const lineHeight = {
  tight: "1.1",
  normal: "1.35",
  /** Long-form reading (the user docs). */
  prose: "1.6",
} as const;

/** Letter spacing (`--eth-tracking-*`). */
export const letterSpacing = {
  normal: "0",
  caps: "0.04em",
  /** Large numeric readouts (transport position). */
  tight: "-0.01em",
} as const;

export const font = {
  /** Inter Variable is bundled (`@fontsource-variable/inter`, imported by `@/kit`). */
  ui: "'Inter Variable', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
  mono: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
} as const;

export const focus = {
  ringWidth: "1px",
  ringOffset: "1px",
  /** Soft halo around a focused transport field. */
  halo: "3px",
} as const;

export const duration = {
  instant: "0ms",
  fast: "80ms",
  normal: "150ms",
  medium: "200ms",
  slow: "300ms",
  tooltipDelay: "500ms",
  /** One cycle of the slow ambient motion behind modals (the backdrop's color blobs). */
  ambient: "14s",
} as const;

export const ease = {
  standard: "cubic-bezier(0.2, 0, 0, 1)",
  out: "cubic-bezier(0, 0, 0.2, 1)",
  in: "cubic-bezier(0.4, 0, 1, 1)",
} as const;

/**
 * Semantic motion: which duration/easing a kind of movement uses (values are the tokens
 * above). CSS reads the same pairs from the component tokens (e.g. `--automation-lane-*`).
 */
export const motion = {
  /** Automation lanes opening/closing: the arrangement's rows slide (layout motion). */
  lane: { duration: duration.slow, ease: ease.standard },
  /** A group folding / unfolding in the arrangement (its rows shrink / grow). */
  fold: { duration: duration.medium, ease: ease.standard },
} as const;

export const opacity = {
  disabled: "0.4",
  /**
   * Translucent surfaces (dialogs, floating panes): how opaque their background is over the
   * blur. Settings > General > Translucency overrides it (`setTranslucencyAmount`).
   */
  translucentSurface: "0.76",
  muted: "0.6",
  /** Piano roll with scale highlight: out-of-scale keys, and unselected out-of-scale notes. */
  scaleOutKey: "0.35",
  scaleOutNote: "0.45",
} as const;

/**
 * Stacking of floating layers: dialogs below popovers (so a Select or Popover opened from a
 * dialog shows above it), tooltips on top. A guided tour interleaves (docs/TUTORIALS.md): its
 * dim sits above dialogs (a target in the settings shows through its hole) and below popovers
 * (a menu opened during a step stays readable); its highlight ring is above popovers (a target
 * inside a menu, or under one, keeps its ring visible) and its bubble above that.
 */
export const zIndex = {
  /** Floating device windows (above the panes, below dialogs and popovers). */
  window: "50",
  dialog: "100",
  tourDim: "150",
  popover: "200",
  tourRing: "240",
  tourBubble: "250",
  tooltip: "300",
} as const;

/** Component and layout dimensions (`--eth-size-*`). */
export const size = {
  controlSm: "18px",
  controlMd: "22px",
  controlLg: "28px",
  /** Icons inline with text follow its font size. */
  iconInline: "1em",
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
  /** How much of a floating window's title bar stays on screen when dragged past an edge. */
  floatingWindowGrip: "120px",
  topBarHeight: "36px",
  /** The thin status bar along the bottom of the editor (desktop and web; "report an issue"). */
  statusBarHeight: "20px",
  /** Floating panes (browser, inspector, editor drawer) and the left icon rail. */
  railWidth: "44px",
  floatGap: "12px",
  /** A collapsed pane's bar: its thickness, and how far it sits from the right edge (clear of the arrangement scrollbar). */
  collapsedBar: "30px",
  collapsedBarInset: "22px",
  /** Blur behind translucent surfaces (dialogs, floating panes). */
  translucentBlur: "28px",
  /** The color blobs drifting behind a modal: their size and blur (they grow with big windows). */
  backdropBlob: "max(500px, 40vmax)",
  backdropBlobBlur: "max(100px, 8vmax)",
  /** Blur of the app behind a modal's backdrop. */
  backdropBlur: "6px",
  browserWidth: "280px",
  inspectorWidth: "320px",
  drawerHeight: "320px",
  floatMinSize: "180px",
  sidebarWidth: "220px",
  sidebarMinWidth: "140px",
  detailHeight: "280px",
  detailMinHeight: "80px",
  /** Room the main view always keeps when the panes around it are resized. */
  mainMinSize: "160px",
  /** Grab width of the pane splitters (laid over the gaps between panes). */
  splitterWidth: "6px",
  popoverMinWidth: "140px",
  selectListMaxHeight: "320px",
  /** Searchable Select (long option lists, e.g. thousands of plugin params): list width. */
  selectSearchWidth: "280px",
  /** "All parameters" list of a device: row height (fits a small knob) and list height. */
  paramListRowHeight: "34px",
  paramListHeight: "420px",
  paramListDialogWidth: "560px",
  /** A device card's inline parameter list scrolls past this height. */
  deviceParamsMaxHeight: "240px",
  /** A factory device window: default width, and its inset from the viewport. */
  deviceWindowWidth: "560px",
  deviceWindowInset: "12px",
  /** The smallest a floating window (kit `FloatingWindow`) can be resized to. */
  windowMinWidth: "240px",
  windowMinHeight: "120px",
  dialogWidth: "420px",
  tooltipMaxWidth: "240px",
  /** Grab width of edge handles (clip and note resize). */
  edgeHandle: "6px",
  /** Editor title in the piano roll / warp toolbars (follows the font size). */
  editorTitleMaxWidth: "16em",
  /** Narrow number inputs in editor toolbars (follows the font size). */
  numberFieldNarrow: "4em",
  /** Status dots: recording, engine status and small "has something" indicators. */
  dotSm: "4px",
  statusDot: "7px",
  statusDotGlow: "6px",
  recordDot: "8px",
  /** What a dialog's chrome (title bar, margins) takes from the viewport height (the news frame's max height). */
  dialogChrome: "12rem",
  // Mobile (`features/mobile`).
  /** The smallest touch target (Apple HIG 44 pt; Android 48 dp). */
  touchTarget: "44px",
  /** Mobile bars: the top bar, the transport row and the slim status row above it. */
  mobileTopBar: "48px",
  mobileDock: "56px",
  mobileStatus: "32px",
  /** Row labels in the mobile track sheet (follows the font size). */
  mobileSheetLabel: "5em",
  /** Mute / solo / arm in a phone's track header (two rows: the name, then these). */
  mobileTrackButton: "34px",
  /** Bottom sheets: as wide as the screen on phones, centred at most this wide on tablets. */
  mobileSheetMaxWidth: "640px",
  /** A selected clip's trim grip on touch screens (drawn; its touch target is 44 px wide). */
  mobileGripWidth: "8px",
  mobileGripHeight: "28px",
  /** The drag badge sits this far above the finger (clear of the thumb). */
  mobileBadgeLift: "48px",
  /** A mixer channel strip on phones, and on tablets. */
  mobileStripWidth: "76px",
  mobileStripWidthWide: "96px",
  // Arrangement.
  /** Default width of the track header column (matches layout.ts HEADER_WIDTH). */
  trackHeaderWidth: "200px",
  /** Track header card: vertical inset, left inset and extra left inset per group depth. */
  trackCardInsetY: "3px",
  trackCardInsetLeft: "6px",
  trackIndent: "12px",
  trackBadge: "26px",
  trackButton: "20px",
  /** Track header buttons (mute, solo, arm, ...), under the volume and pan. */
  trackHeaderButton: "20px",
  trackMeterWidth: "3px",
  trackFaderWidth: "44px",
  trackFaderThumb: "8px",
  /** Blur of the glow around a selected track header. */
  trackSelectedGlow: "10px",
  /** Two-layer halo around selected clips and notes (inner and outer blur). */
  selectedGlowInner: "9px",
  selectedGlowOuter: "24px",
  /** "+ Add track" row below the tracks: height and horizontal padding. */
  addTrackHeight: "32px",
  /** The pan knob in a track header (HeaderPan). */
  trackPanKnob: "18px",
  /** A compact (short) track header: a smaller pan knob and a short fader. */
  trackPanKnobCompact: "14px",
  trackFaderWidthCompact: "30px",
  /** The piano roll's key-height grip, along the keyboard's left edge. */
  pianoZoomGrip: "8px",
  /** The step sequencer (Drum Rack tracks): row height, pad-name column, cell width. */
  beatRowHeight: "28px",
  beatNameWidth: "120px",
  beatRulerHeight: "20px",
  /** The sidechain source / tap selects in a device header: capped, the name keeps room. */
  sidechainSelectMaxWidth: "96px",
  drumPadWidth: "92px",
  drumPadHeight: "70px",
  addTrackPadLeft: "18px",
  addTrackPadRight: "20px",
  /** Piano-roll scale highlight: accent mark on in-scale keys, and the wider one on roots. */
  pianoScaleMark: "3px",
  pianoScaleRootMark: "5px",
  clipTitleHeight: "14px",
  clipTitleLineHeight: "12px",
  // Timeline ruler.
  rulerLoopHeight: "4px",
  /** Extra invisible grab area below the loop brace. */
  rulerLoopHitExtra: "3px",
  rulerLabelTop: "6px",
  rulerLabelLineHeight: "10px",
  playheadMarkerHalfWidth: "5px",
  playheadMarkerHeight: "7px",
  // Piano roll.
  gridStepLabelMinWidth: "3.5em",
  loopBarHeight: "8px",
  /** Note-stretch bar on the piano-roll ruler: height, edge marker width, extra grab area below. */
  noteStretchBarHeight: "6px",
  noteStretchEdgeWidth: "4px",
  noteStretchHitExtra: "4px",
  velocityBarWidth: "4px",
  // Browser, plugins, project, recording, transport.
  browserRowHeight: "24px",
  emptyStateIcon: "22px",
  /** Project screen (dialog) width, its big "New project" button and list rows. */
  projectScreenWidth: "560px",
  /** The welcome screen (the project screen's home): get started | recent projects. */
  welcomeWidth: "880px",
  /** The download page (`/download`): its column, the big download button, a platform card. */
  downloadPageWidth: "1040px",
  downloadPrimaryHeight: "64px",
  downloadCardMinWidth: "260px",
  /** The user docs (`/docs`): the section nav, the reading column and the on-page contents. */
  docsNavWidth: "264px",
  docsContentWidth: "760px",
  docsTocWidth: "220px",
  docsSearchWidth: "420px",
  /** The welcome screen's search over the recent projects. */
  projectSearchWidth: "180px",
  projectNewHeight: "72px",
  projectNewIcon: "20px",
  projectRowHeight: "32px",
  projectInputHeight: "20px",
  recordPanelMinWidth: "320px",
  transportIcon: "15px",
  // Device cards.
  deviceTrackPickerMinWidth: "120px",
  deviceAddMinWidth: "140px",
  deviceChainEndMinWidth: "48px",
  deviceCardMinWidth: "148px",
  deviceParamMinWidth: "52px",
  /** Stacked device card: knob diameter range for main (lg) and folded (md) controls. */
  deviceKnobLgMin: "34px",
  deviceKnobLgMax: "54px",
  deviceKnobMin: "26px",
  deviceKnobMax: "38px",
  /** Room beside a knob in a parameter cell: main, folded and narrow cards. */
  deviceParamRoom: "22px",
  deviceParamRoomMore: "26px",
  deviceParamRoomNarrow: "14px",
  deviceGroupMinWidth: "220px",
  deviceGroupMinWidthWide: "200px",
  // Audio settings dialog.
  audioSettingsDialogWidth: "520px",
  audioSettingsMeterNameWidth: "120px",
  // Guided tours (features/tutorials).
  /** The tour's step bubble (narrower on small screens), and the most an illustration in it takes. */
  tourBubbleWidth: "380px",
  tourIllustrationMaxHeight: "120px",
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

/**
 * Icon/text ink on a colored fill (track badges, clip titles), whichever contrasts more
 * (`--eth-ink-<key>`). Theme-independent: the fill is a track color.
 */
export const ink = {
  dark: "#14161b",
  light: "#ffffff",
} as const;

/** Pure white/black to tint or shade a color with `color-mix()` (`--eth-mix-<key>`). */
export const mix = {
  white: "#ffffff",
  black: "#000000",
} as const;

/** Track/clip color palette: soft, luminous hues (`--eth-track-<i>`). Index with `trackColor(i)`. */
export const TRACK_COLORS = [
  "#e8919d",
  "#e8a585",
  "#e6c07e",
  "#d4d08a",
  "#a9cf8b",
  "#86cfa8",
  "#7cc6c0",
  "#86bfe0",
  "#8fa8e6",
  "#a39ae8",
  "#bd9ae3",
  "#d99adf",
  "#e59abf",
  "#c7b8a8",
  "#a4b6a9",
  "#d6d9e2",
] as const;

/**
 * The color shown for a track or clip whose color is unknown (a mixed selection, a track
 * that isn't shown): a quiet neutral, outside the palette (`--eth-track-fallback`).
 */
export const TRACK_FALLBACK_COLOR = "#8a91a8";

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
    // Flat: no border, a light grey fill (the text color at low opacity, so it works in
    // both themes).
    "--button-border": "transparent",
    "--button-bg": "color-mix(in srgb, var(--eth-color-text) 10%, transparent)",
    "--button-bg-hover": "color-mix(in srgb, var(--eth-color-text) 16%, transparent)",
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
    "--knob-track-hover": v("color-border-strong"),
    "--knob-value": v("color-accent"),
    "--knob-value-hover": v("color-accent-hover"),
    // A ring with a dot at the value (no center pointer line); set a color to bring it back.
    "--knob-pointer": "transparent",
    "--knob-dot": v("color-text"),
    "--knob-center-fg": v("color-text"),
    "--knob-center-font-size": v("fs-xs"),
    "--knob-center-inset": v("space-xs"),
    "--knob-readout-fg": v("color-text"),
    "--knob-stroke": v("size-knob-stroke"),
    "--knob-pointer-width": v("size-knob-pointer"),
    "--knob-pointer-linecap": "round",
    "--knob-linecap": "round",
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
    "--panel-border": v("color-border-light"),
    "--panel-radius": v("radius-lg"),
    "--panel-header-height": v("size-panel-header-height"),
    "--panel-header-bg": v("color-bg-panel"),
    "--panel-header-fg": v("color-text-dim"),
    "--panel-header-font-size": v("fs-sm"),
    "--panel-header-transform": "none",
    "--panel-header-tracking": v("tracking-normal"),
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
    "--tab-fg-active": v("color-text"),
    "--tab-bg-hover": v("color-bg-hover"),
    "--tab-bg-active": v("color-bg-active"),
    "--tab-radius": v("radius-pill"),
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
    // Context menu entrance: grows out of the click point.
    "--context-menu-enter-duration": v("duration-fast"),
    "--context-menu-enter-ease": v("ease-out"),
    "--context-menu-enter-scale": "0.92",
    "--select-exit-duration": v("duration-fast"),
    "--popover-bg": v("color-bg-overlay"),
    "--popover-border": v("color-border-light"),
    "--popover-radius": v("radius-lg"),
    "--popover-shadow": v("shadow-lg"),
    "--menu-item-height": v("size-row-height"),
    "--menu-item-bg-hover": v("color-accent-subtle"),
  },
  automation: {
    // Automation lanes opening/closing: same timing as `motion.lane` (the row heights).
    "--automation-lane-duration": v("duration-slow"),
    "--automation-lane-ease": v("ease-standard"),
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
  ink,
  mix,
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
  out.push(["--eth-track-fallback", TRACK_FALLBACK_COLOR]);
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
