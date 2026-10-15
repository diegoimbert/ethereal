# Brand assets

- `ethereal-logo.svg`: the logo, the source of truth (840 × 840, transparent). The app draws
  it inline with `Logo` (`ui/src/brand/Logo.tsx`); the telemetry admin pages inline it too
  (`services/telemetry/src/adminPage.ts`). Tests in both places check that their copies match
  this file.
- `ethereal-mark-small.svg`: a simplified crystal (five flat facets) for 16 to 24 px, where the
  logo's facets blur together. Used for `favicon.svg`, the 16 and 24 px frames of the `.ico`
  files and the admin site's favicon.
- `generate.mjs`: renders every icon from the two files above.
- `preview/`: contact sheets of every generated icon on the dark and the light background
  (`contact-sheet.png`: 16 to 1024 px at half scale; `contact-small.png`: 16 to 64 px at 1:1).

## Regenerating the icons

After changing either SVG:

```sh
deps=$(mktemp -d) && (cd "$deps" && npm init -y >/dev/null && npm i @resvg/resvg-js@2 sharp@0.35)
node assets/brand/generate.mjs "$deps"
```

It writes:

| Where | What |
| --- | --- |
| `apps/desktop/src-tauri/icons/` | `tauri icon` output from the transparent crystal (8 % padding): PNGs (Linux), `Square*Logo.png` / `StoreLogo.png` (Windows Store), and `icon.ico` (Windows; the small mark at 16 and 24 px). `icon.icns` (macOS) comes from the plated variant: the Big Sur grid, an 824 px rounded plate on the 1024 canvas with a soft shadow, the app's dark background lifted toward indigo, the crystal at 70 % of the plate. |
| `apps/web/public/` | `favicon.ico` (16, 32, 48), `favicon.svg` (the small mark), `apple-touch-icon.png` (180, full-bleed plate), `icon-192.png` / `icon-512.png` (rounded plate) and `icon-maskable-512.png` (full bleed, the crystal inside the safe zone), listed in `manifest.webmanifest`. |
| `assets/brand/preview/` | the contact sheets. |

The desktop icons ship with the next release. There is no tray or menu-bar icon.
