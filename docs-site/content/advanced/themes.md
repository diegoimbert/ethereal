---
title: themes
order: 85
summary: Pick one of the six built-in themes, follow your system's light and dark mode, or make, share and import your own.
platform: all
audience: everyone
---

A theme sets every color of the interface: the panels, the text, the accent, the arrangement, the meters and the waveforms. Pick one in the settings, under **appearance** › **theme**. It applies at once and is kept on this device.

## built-in themes

Click a card to use its theme:

| theme | what it looks like |
|---|---|
| **dark** | soft charcoal with an orchid accent (the default) |
| **obsidian** | near black with a cyan accent, for dim rooms and OLED screens |
| **ember** | warm brown charcoal with an amber accent |
| **tide** | deep navy with a periwinkle accent |
| **dusk** | softer slate surfaces and gentler text, with a rose accent |
| **light** | light grey with an orchid accent |

Every built-in theme keeps text and controls readable: they all pass the WCAG AA contrast levels.

The **switch to light theme** and **switch to dark theme** commands in the [command palette](/docs/basics/command-palette-and-settings) switch to the light or dark theme you used last.

## following the system

Turn on **follow the system's light or dark** to switch themes when your system does (for example at sunset). Pick the theme to use for each under **light theme** and **dark theme**. Clicking a theme card turns following off again.

## make your own theme

Click **create your own theme**: it starts as a copy of the theme you're using. In the editor:

- **name**: what its card shows.
- **base**: dark or light. A light base also turns off the glow and uses lighter shadows.
- **main colors**: the background, panels, raised surfaces, text, dim text, accent, and the success, warning, danger and information colors. Click a swatch to pick a color, or type one (`#1c1f26`, `rgba(…)`).
- **all tokens**: every other color of the app. Each is worked out from the main colors (hover shades, borders, the accent's lighter and dimmer tones, the meters…) until you set it here; the reset button gives it back.
- **contrast**: what in your theme fails the WCAG AA levels, such as dim text too faint on the panels. Fix those so everything stays readable.

The whole app shows your changes while you edit, the arrangement and the meters included. **save** keeps the theme and uses it; **cancel** goes back to the theme you had.

Your themes get a card next to the built-in ones. Its **…** button (or a right click) can **edit**, **duplicate**, **export…** or **delete** it. The built-in themes can be duplicated and exported too.

> [!note]
> Your themes are kept on this device: in this browser on the web, in the app on the desktop. Export them to keep a copy or use them elsewhere.

## import and export

**export…** saves a theme as a `.json` file: the browser downloads it, the desktop app asks where to save it. Share it like any file. **import theme…** reads one back: the theme is added to your themes and used at once.

A theme file names its base and some colors. Ethereal works out the colors a file leaves out, and skips (and tells you about) colors it doesn't know or values that aren't colors. A file made by a newer version of Ethereal can't be imported: update Ethereal first.

## where next

- [settings reference](/docs/advanced/settings#appearance)
- [the theme file format](https://github.com/diegoimbert/ethereal/blob/main/docs/THEMES.md#theme-files)
- [the command palette and settings](/docs/basics/command-palette-and-settings)
