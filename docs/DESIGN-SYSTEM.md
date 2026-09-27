# Ethereal design system

You can restyle the whole app (colors, control shapes, typography, spacing, motion) from **one
file**: `ui/src/theme/tokens.ts`. Every kit component and the app shell read their look only from
the tokens in that file.

```
ui/src/theme/
  tokens.ts            ← the single source: edit this
  tokens.css           ← GENERATED CSS custom properties (never edit by hand)
  css.ts / gen-css.ts  ← generator (tokens.ts → tokens.css)
  base.css             ← global reset/body styles (tokens only)
  index.ts             ← runtime API: cssVar, readToken, trackColor, setTheme/useTheme
  hardcoded.ts         ← hard-coded value scanner (test + report)
ui/src/kit/            ← components (Button, Knob, Fader, ...); kit.css reads tokens only
ui/src/features/kit-gallery/  ← visual gallery of every component, variant and token
```

## Workflow

1. Run `just dev-ui` and open `http://localhost:<port>/?kit` (the port is printed by the
   recipe, or `just dev-port`). The gallery shows every component in every size/tone and every
   token, with a dark/light switch. `?kit` removed = the app.
2. Edit `ui/src/theme/tokens.ts`.
3. Regenerate the CSS: `node ui/src/theme/gen-css.ts` (Node ≥ 23.6; Vite hot-reloads it).
4. `pnpm --filter @ethereal/ui test` — `tokens.test.ts` fails if `tokens.css` is stale.

For quick experiments you can also override any custom property in the browser devtools on
`<html>`; copy the value back into `tokens.ts` when happy.

## How to change…

| I want to change… | Edit in `tokens.ts` |
| --- | --- |
| The accent color | `darkColors.accent` / `accentHover` / `accentDim` / `accentSubtle` (and `lightColors`) |
| Backgrounds / panel levels | `bg`, `bgInset`, `bgPanel`, `bgRaised`, `bgHover`, `bgActive`, `bgOverlay` |
| Text colors | `text`, `textDim`, `textDisabled`, `textInverse` |
| Status colors | `ok`, `warn`, `danger`, `info`, `record`, `play` |
| Meter colors / gradient | `meterLow/Mid/High` colors, `meter.stopMid/stopHigh` positions |
| Track/clip palette | `TRACK_COLORS` |
| Fonts | `font.ui`, `font.mono` (load web fonts from `base.css` via `@font-face` if needed) |
| Font sizes / weights | `fontSize`, `fontWeight`, `lineHeight`, `letterSpacing` |
| Spacing rhythm | `space` |
| Roundness of everything | `radius` (e.g. set `sm`/`md` to `0` for a square look) |
| Button height / row heights | `size.controlSm/Md/Lg`, `size.rowHeight*` |
| Knob diameter / stroke | `size.knobSm/Md/Lg`, `size.knobStroke`, `size.knobPointer` |
| Fader width / thumb | `size.faderWidth`, `faderTrackWidth`, `faderThumbHeight` |
| Shell layout | `size.sidebarWidth`, `detailHeight`, `topBarHeight` |
| Shadows | `darkShadows` / `lightShadows` |
| Animation speed | `duration`, `ease` (reduced-motion users get `duration.instant`) |
| One component only | `componentTokens.<component>` (see below) |

### Component tokens (per-component shape and color)

Each kit component reads its own tokens, which default to global tokens:

- **Button / IconButton**: `--button-height`, `--button-padding-x`, `--button-font-size`,
  `--button-radius`, `--button-border`, `--button-bg`, `--button-bg-hover`, `--button-fg`,
  `--button-accent-bg(-hover)`, `--button-accent-fg`, `--button-danger-bg`, `--button-danger-fg`,
  `--button-ghost-bg-hover`
- **Knob**: `--knob-size`, `--knob-track`, `--knob-value`, `--knob-pointer`, `--knob-stroke`,
  `--knob-pointer-width`, `--knob-linecap` (`butt`/`round`), `--knob-body`, `--knob-body-border`
  (fill/outline of a round body behind the arc: set them for a "cap" knob), `--knob-label-fg`
- **Fader**: `--fader-width`, `--fader-track-width`, `--fader-track-bg`, `--fader-track-radius`,
  `--fader-fill`, `--fader-thumb-height`, `--fader-thumb-bg`, `--fader-thumb-border`,
  `--fader-thumb-radius` (e.g. `var(--eth-radius-pill)` for a round thumb)
- **Meter**: `--meter-channel-width`, `--meter-gap`, `--meter-bg`, `--meter-radius`,
  `--meter-low/mid/high`, `--meter-stop-mid/high`, `--meter-clip-height`
- **Panel**: `--panel-bg`, `--panel-border`, `--panel-radius`, `--panel-header-height`,
  `--panel-header-bg`, `--panel-header-fg`, `--panel-header-font-size`, `--panel-header-transform`
- **Tabs**: `--tab-fg`, `--tab-fg-active`, `--tab-bg-hover`, `--tab-bg-active`, `--tab-radius`
- **Toggle**: `--toggle-width`, `--toggle-height`, `--toggle-track-off/on`, `--toggle-thumb`, `--toggle-radius`
- **TextInput / Select / NumberField**: `--input-height`, `--input-padding-x`, `--input-font-size`,
  `--input-bg`, `--input-fg`, `--input-border`, `--input-border-focus`, `--input-radius`
- **Popover / Menu**: `--popover-bg`, `--popover-border`, `--popover-radius`, `--popover-shadow`,
  `--menu-item-height`, `--menu-item-bg-hover`
- **Dialog**: `--dialog-width`, `--dialog-bg`, `--dialog-border`, `--dialog-radius`,
  `--dialog-shadow`, `--dialog-backdrop`
- **Tooltip**: `--tooltip-bg`, `--tooltip-fg`, `--tooltip-border`, `--tooltip-radius`, `--tooltip-delay`
- **Badge**: `--badge-radius`, `--badge-bg`, `--badge-fg`, `--badge-font-size`

Change a default for the whole app in `componentTokens` (then regenerate). To restyle one area
only, set the token on a container in that feature's CSS:
`.eth-strip { --knob-value: var(--eth-color-ok); }`.

Size variants (`sm`/`lg`) re-point the component token to the matching global size token
(e.g. `.eth-button--sm { --button-height: var(--eth-size-control-sm) }`), so resize via the
global `size.*` tokens.

## Token reference

CSS names are derived from `tokens.ts`: `--eth-<group>-<kebab-key>`.

| Group (TS export) | CSS prefix | Theme-dependent |
| --- | --- | --- |
| `darkColors` / `lightColors` | `--eth-color-*` | yes |
| `darkShadows` / `lightShadows` | `--eth-shadow-{sm,md,lg,inset}` | yes |
| `space` | `--eth-space-{0,xs,sm,md,lg,xl,2xl,3xl}` | no |
| `radius` | `--eth-radius-{none,sm,md,lg,xl,pill,round}` | no |
| `border` | `--eth-border-{width,width-strong}` | no |
| `fontSize` | `--eth-fs-{xs,sm,md,lg,xl,2xl}` | no |
| `fontWeight` | `--eth-fw-{regular,medium,bold}` | no |
| `lineHeight` | `--eth-lh-{tight,normal}` | no |
| `letterSpacing` | `--eth-tracking-{normal,caps}` | no |
| `font` | `--eth-font-{ui,mono}` | no |
| `focus` | `--eth-focus-{ring-width,ring-offset}` (color: `--eth-color-focus`) | no |
| `duration` | `--eth-duration-{instant,fast,normal,slow,tooltip-delay}` | no |
| `ease` | `--eth-ease-{standard,out,in}` | no |
| `opacity` | `--eth-opacity-{disabled,muted}` | no |
| `zIndex` | `--eth-z-{popover,tooltip,dialog}` | no |
| `size` | `--eth-size-*` (controls, icons, knob, fader, meter, toggle, rows, shell, overlays) | no |
| `meter` | `--eth-meter-{stop-mid,stop-high}` | no |
| `TRACK_COLORS` | `--eth-track-{0..15}` | no |
| `componentTokens` | `--button-*`, `--knob-*`, ... | follow theme |

The gallery (`?kit`) lists every token with its current value.

## Themes

Dark is the default; light ships alongside. The theme is the `data-theme` attribute on `<html>`
(`dark` | `light`), remembered in `localStorage` (`eth-theme`) and applied when `@/kit` loads.

- Switch: `setTheme("light")`, or `const [theme, setTheme] = useTheme()` in React.
- A subtree can use another theme: `<div data-theme="light">…</div>`.
- Add a theme: add a `ColorTokens` + `ShadowTokens` pair and an entry in `themes` in
  `tokens.ts`, regenerate. The test checks every theme defines the same keys.

## Components

All from `@/kit`. Shared variant vocabulary (`ui/src/kit/variants.ts`):
`size: "sm" | "md" | "lg"`, `tone: "default" | "accent" | "danger" | "ghost"`,
status tones (Badge) `"default" | "accent" | "ok" | "warn" | "danger"`.

| Component | Notes |
| --- | --- |
| `Button` | `tone`, `size`, `active` (aria-pressed). `variant` is deprecated (`primary` → `accent`). |
| `IconButton` | glyph/icon only; `label` is required (accessible name + title). |
| `Knob` | `size` = `sm`/`md`/`lg` or pixels; `bipolar`; drag/keys/double-click reset. |
| `Fader` | vertical; `height` in px (drives drag sensitivity); width from tokens. |
| `Meter` | dB peak meter; colors/stops/widths from `--meter-*`. |
| `Panel` | titled container with header actions. |
| `Tabs` | `role=tablist`; arrow-key navigation; content is yours. |
| `Toggle` | `role=switch`. |
| `TextInput`, `Select`, `NumberField` | share `--input-*` tokens; NumberField commits on Enter/blur, ↑/↓ step (Shift ×10), Esc reverts. |
| `Popover`, `Menu` | anchored to a trigger render-prop; close on outside click/Escape. |
| `Dialog` | modal in a portal; Escape/backdrop closes. |
| `Tooltip` | hover/focus; delay = `--tooltip-delay`. |
| `Badge` | status label/count. |

## Rules for feature code

1. **Use kit components** for buttons, knobs, faders, meters, panels, tabs, toggles, inputs,
   menus, dialogs and tooltips. If the kit lacks something, ask for it (BCR to
   `design-system`) instead of styling a one-off.
2. **Only tokens in CSS**: `var(--eth-color-*)`, `var(--eth-space-*)`, `var(--eth-fs-*)`,
   `var(--eth-radius-*)`, `var(--eth-size-*)`… No hex/rgb colors, no px font sizes, and
   prefer size/space tokens over px lengths. Layout-specific dimensions that are not reusable
   may stay local, but consider adding a `size.*` token.
3. **Inline styles**: use `cssVar("accent")` → `var(--eth-color-accent)`, or
   `var(--eth-track-<i>)`.
4. **Canvas / WebGL** (no CSS vars): read computed values with `readToken("--eth-color-text")`
   and re-read on theme change (`useTheme()` re-renders). Raw `TRACK_COLORS` are fine for data.
5. Customize a kit component in a feature by setting its component tokens on a container, not
   by overriding `.eth-*` internals.

### Enforcement

- `ui/src/theme/hardcoded.test.ts` fails on literal colors, px font sizes or px lengths in
  `ui/src/kit`, `ui/src/app` and `ui/src/features/kit-gallery`.
- `node ui/src/theme/report-hardcoded.ts [dir…]` lists what remains in `ui/src/features`
  (inventory for the design sweep; always exits 0).
