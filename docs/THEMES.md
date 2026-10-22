# Themes

A theme sets every color token of the UI (`ui/src/theme/tokens.ts`, `--eth-color-*` and
`--eth-shadow-*`). The sizes, spacing, fonts and motion tokens are the same in every theme.
See [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) for the token system itself.

## Model

A theme is a **definition** (`ThemeDefinition`, `ui/src/theme/themeModel.ts`):

- `id`: unique. A built-in's name (`dark`, `obsidian`, ...) or `custom-…` for a user's theme.
- `name`: what the picker shows (built-ins show a translated name instead).
- `base`: `dark` or `light`. It sets CSS `color-scheme`, the shadows, the glow (none in a light
  theme) and which light-only style rules apply (`[data-theme="light"]` in feature CSS).
- `colors`: some color tokens, keyed by their `tokens.ts` name (`bgPanel`, `accent`, ...).

Resolving a definition (`resolveColors`): the **main colors** it leaves out come from its base
scheme's built-in theme (`dark` or `light`), every other token is **derived** from the main
colors (`deriveColors`), then any token the definition gives wins over the derived one.

### Main colors

| token | CSS property | used for |
|---|---|---|
| `bg` | `--eth-color-bg` | the app background, the arrangement |
| `bgPanel` | `--eth-color-bg-panel` | panels, dialogs, clip bodies |
| `bgRaised` | `--eth-color-bg-raised` | raised surfaces: menus, cards, the top bar |
| `text` | `--eth-color-text` | text and icons |
| `textDim` | `--eth-color-text-dim` | secondary text, labels |
| `accent` | `--eth-color-accent` | the accent: selection, knob values, primary buttons, focus |
| `ok` | `--eth-color-ok` | success, play, the meter's low range |
| `warn` | `--eth-color-warn` | warnings, the meter's middle range |
| `danger` | `--eth-color-danger` | errors, record, the meter's top range |
| `info` | `--eth-color-info` | information, the warp editor's waveform |

### Derived tokens

`mix(a p, b)` is `color-mix(in srgb, a p, b)`; `a / α` is `a` at opacity α. Values for a dark
base, then a light one where they differ.

| token | derived as |
|---|---|
| `bgInset` | `mix(bg 78%, black)`; light `mix(bg 96%, text)` |
| `bgCard` | `mix(text 4%, bgRaised)`; light `mix(white 60%, bgRaised)` |
| `bgHover`, `bgActive` | `mix(text 5%, bgRaised)`, `mix(text 10%, bgRaised)` |
| `bgOverlay` | `bgRaised / 0.96` (light 0.97) |
| `backdrop` | `mix(bg 50%, black) / 0.5`; light `text / 0.22` |
| `border` | `mix(bg 65%, black)`; light `mix(text 14%, bgPanel)` |
| `borderLight` | `mix(text 5.5%, bgRaised)`; light `mix(text 8%, bgPanel)` |
| `borderStrong` | `mix(text 15%, bgRaised)`; light `mix(text 25%, bgPanel)` |
| `borderCard` | `mix(text 8%, bgCard)` |
| `textDisabled` | `mix(text 33%, bg)` |
| `textInverse` | text on the accent: whichever of near-black, white or a darkened `bg` contrasts most with `accent` |
| `accentHover` | `mix(white 15%, accent)` (light 10%) |
| `accentDim` | `mix(accent 64%, bg)`; light `mix(accent 38%, bgRaised)` |
| `accentSubtle` | `accent / 0.14` (light 0.12) |
| `accentGlow` | `accent`, saturation +28 %, lightness −7 % (light −5 %) |
| `selection`, `focus` | `accent` |
| `playhead` | `mix(white 30%, text)`; light `text` |
| `playheadPlaying`, `playheadGlow` | `accent`, `accent / 0.35`; light `text`, `transparent` |
| `record`, `play` | `danger`, `ok` |
| `meterLow`, `meterMid`, `meterHigh` | `ok`, `warn`, `danger` |
| `knobTrack` | `borderLight` (light `mix(text 14%, bgPanel)`) |
| `outsideShade` | `rgba(0, 0, 0, 0.35)`; light `text / 0.08` |
| `warpWave`, `warpWaveDim` | `info / 0.85`, `info / 0.3` |
| `pianoScaleRow`, `pianoScaleRootRow` | `mix(accent 6%, bgPanel)`, `mix(accent 14%, bgPanel)` |
| `pianoScaleRootLine`, `pianoScaleKeyMark` | `accent / 0.35`, `accent / 0.55` |
| `clipSeam`, `piano*Key*` | the base scheme's built-in values |

## Built-in themes

`ui/src/theme/builtinThemes.ts`. `dark` (the default) and `light` spell out every token
(`darkColors` / `lightColors` in `tokens.ts`, hand-tuned); the others give only their main
colors.

| id | base | accent | character |
|---|---|---|---|
| `dark` | dark | `#d48fd0` orchid | soft charcoal (default) |
| `obsidian` | dark | `#3fc8e4` cyan | near black, neutral |
| `ember` | dark | `#f0a35e` amber | warm brown charcoal |
| `tide` | dark | `#8aa4ff` periwinkle | deep navy |
| `dusk` | dark | `#ec9bb0` rose | soft: lighter slate, gentler text |
| `light` | light | `#ab55a1` orchid | light grey |

To add one, add its definition there and run `just gen-tokens`: `tokens.css` declares every
built-in (`[data-theme-id="…"]` blocks after the scheme's), so switching needs no script.

## Contrast

`CONTRAST_CHECKS` (`themeModel.ts`) are the WCAG AA requirements every built-in passes
(`themeModel.test.ts`): `text` on every surface and `textDim` on the main ones at 4.5:1,
`textInverse` on `accent` (primary buttons) at 4.5:1, and the accent, focus, status colors and
the playhead against `bg`, `bgPanel` and `bgRaised` at 3:1 (non-text contrast).
`contrastFailures(colors)` lists what a theme fails.

## Runtime

`ui/src/theme/themeRuntime.ts` (re-exported by `@/theme`):

- `<html data-theme>` is the applied theme's base (`dark` / `light`); `getTheme()` /
  `useTheme()` return it, `setTheme(scheme)` applies the theme last used for that scheme.
- `<html data-theme-id>` is the theme: `getThemeId()` / `useThemeId()` / `setThemeId(id)`.
  A user's theme (and the theme editor's preview, id `preview`) is written into
  `<style id="eth-theme-custom">`.
- `<html data-theme-rev>` changes on every applied change. Canvas code redraws on
  `useThemeKey()` (or `useThemeColor(key)`, which follows it); the arranger's WebGL layer
  observes the three attributes.
- `setThemeMode("system")` follows `prefers-color-scheme`, with `setSchemeTheme(scheme, id)`
  picking the theme for each.
- Stored in `localStorage` on this device (the desktop app's webview keeps it in its own
  profile): `eth-theme` (the scheme), `eth-theme-id`, `eth-theme-dark`, `eth-theme-light`,
  `eth-theme-mode` and `eth-custom-themes` (the user's definitions, a JSON array).

## The user's themes

Settings > Appearance (`ui/src/features/audio-settings/ThemePicker.tsx`): **create your own
theme** copies the applied theme (a built-in's main colors, all of a user theme's colors) into
the editor (`ThemeEditor.tsx`), which previews the draft on the whole app (`previewTheme`) and
lists the contrast checks it fails. A card's menu edits, duplicates, exports or deletes.
Export and import (`themeFiles.ts`) download / pick a file in the browser; the desktop app
saves and opens it through its native dialogs (`text_file_save` / `text_file_open`,
`apps/desktop/src-tauri/src/text_file.rs`), as its webview can do neither.

## Theme files

A theme file is JSON (`serializeTheme`, `parseThemeFile` in `themeModel.ts`):

```json
{
  "format": "ethereal-theme",
  "version": 1,
  "name": "midnight",
  "base": "dark",
  "colors": {
    "bg": "#0a0a0c",
    "accent": "#3fc8e4"
  }
}
```

- `format` must be `ethereal-theme`; `version` is an integer, at most `THEME_VERSION` (1).
  A newer version is refused (made by a newer Ethereal).
- `base` is required: `dark` or `light`.
- `colors` holds any color tokens by their `tokens.ts` name, as `#rgb`, `#rrggbb`, `#rrggbbaa`,
  `rgb()`/`rgba()` or `transparent`. What's missing is taken from the base or derived. Unknown
  tokens and values that aren't colors are dropped (and reported); a missing `name` gets a default one.
- Version 1 readers ignore other top-level fields. A change that older readers would misread
  bumps `version`.
