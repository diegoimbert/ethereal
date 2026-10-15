// Regenerates every icon from the logo (assets/brand/README.md).
//
//   node assets/brand/generate.mjs <deps-dir>
//
// <deps-dir>: a folder where `npm i @resvg/resvg-js@2 sharp@0.35` was run (kept out of the
// repo's dependencies: this runs only when the logo changes). Also runs `tauri icon`.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const depsDir = process.argv[2];
if (!depsDir) {
  console.error("usage: node assets/brand/generate.mjs <deps-dir with @resvg/resvg-js and sharp installed>");
  process.exit(1);
}
const require = createRequire(join(resolve(depsDir), "package.json"));
const { Resvg } = require("@resvg/resvg-js");
const sharp = require("sharp");

const brand = dirname(fileURLToPath(import.meta.url));
const root = resolve(brand, "../..");
const tauriIcons = join(root, "apps/desktop/src-tauri/icons");
const webPublic = join(root, "apps/web/public");

// ---- The artwork ---------------------------------------------------------------------------

/** An SVG's content (between <svg> and </svg>), its gradient ids prefixed so two can share a file. */
const body = (file, prefix) =>
  readFileSync(join(brand, file), "utf8")
    .replace(/^[\s\S]*?<svg[^>]*>/, "")
    .replace(/<\/svg>\s*$/, "")
    .replace(/id="([^"]+)"/g, `id="${prefix}$1"`)
    .replace(/url\(#([^)]+)\)/g, `url(#${prefix}$1)`);
const LOGO = body("ethereal-logo.svg", "l-");
const SMALL = body("ethereal-mark-small.svg", "s-");

/** The crystal (840 × 840 artwork, centered) scaled to `h` px tall, centered at (cx, cy). */
const crystal = (art, cx, cy, h) => `<g transform="translate(${cx - h / 2} ${cy - h / 2}) scale(${h / 840})">${art}</g>`;
const svg = (size, content) => `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${content}</svg>`;

// The plate: the app's dark background (--eth-color-bg #16181e) lifted toward a deep indigo at
// the top, with a faint violet glow behind the crystal and a hairline highlight on the edge.
const PLATE_DEFS = `
<linearGradient id="plate" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#282c40"/><stop offset="1" stop-color="#14161c"/></linearGradient>
<radialGradient id="glow" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#9986f5" stop-opacity="0.22"/><stop offset="1" stop-color="#9986f5" stop-opacity="0"/></radialGradient>
<linearGradient id="edge" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff" stop-opacity="0.14"/><stop offset="1" stop-color="#ffffff" stop-opacity="0.03"/></linearGradient>
<filter id="shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000000" flood-opacity="0.32"/></filter>`;

/** A plate at (x, y), `w` wide, corner radius `r` (0: full bleed), the crystal at `art` of its height. */
function plated(size, { x, y, w, r, shadow, art, mark = LOGO }) {
  const rect = (extra) => `<rect x="${x}" y="${y}" width="${w}" height="${w}" rx="${r}" ${extra}/>`;
  const edge = r > 0 ? rect(`fill="none" stroke="url(#edge)" stroke-width="${Math.max(1, w / 412)}"`) : "";
  return svg(
    size,
    `<defs>${PLATE_DEFS}</defs>` +
      rect(`fill="url(#plate)"${shadow ? ` filter="url(#shadow)"` : ""}`) +
      `<circle cx="${x + w / 2}" cy="${y + w / 2}" r="${w * 0.42}" fill="url(#glow)"/>` +
      edge +
      crystal(mark, x + w / 2, y + w / 2, w * art),
  );
}

const VARIANTS = {
  // macOS (Big Sur+ grid): an 824 px plate on the 1024 canvas, corner ≈ 22.5 %, a soft shadow.
  macos: plated(1024, { x: 100, y: 100, w: 824, r: 185, shadow: true, art: 0.7 }),
  // Windows / Linux: the bare crystal with 8 % padding.
  transparent: svg(1024, crystal(LOGO, 512, 512, 1024 * 0.84)),
  // The simplified mark for 16–24 px (the facets turn to mush there).
  small: svg(1024, crystal(SMALL, 512, 512, 1024 * 0.9)),
  // Web: full-bleed squares (iOS and Android cut their own shapes).
  appleTouch: plated(1024, { x: 0, y: 0, w: 1024, r: 0, art: 0.66 }),
  // Maskable: the crystal inside the 80 % safe circle.
  maskable: plated(1024, { x: 0, y: 0, w: 1024, r: 0, art: 0.58 }),
  // PWA "any": the rounded plate, no shadow.
  pwa: plated(1024, { x: 32, y: 32, w: 960, r: 216, art: 0.68 }),
};

// ---- Rendering -----------------------------------------------------------------------------

/** One variant rendered at `px`, optimized. */
async function png(variant, px) {
  const raw = new Resvg(VARIANTS[variant], { fitTo: { mode: "width", value: px } }).render().asPng();
  return sharp(raw).png({ compressionLevel: 9, adaptiveFiltering: true }).toBuffer();
}

/** A .ico of PNG frames (Vista+ reads PNG entries; every browser does). */
function ico(frames) {
  const head = Buffer.alloc(6 + 16 * frames.length);
  head.writeUInt16LE(0, 0);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(frames.length, 4);
  let offset = head.length;
  frames.forEach(({ size, data }, i) => {
    const e = 6 + 16 * i;
    head.writeUInt8(size >= 256 ? 0 : size, e);
    head.writeUInt8(size >= 256 ? 0 : size, e + 1);
    head.writeUInt16LE(1, e + 4); // planes
    head.writeUInt16LE(32, e + 6); // bits per pixel
    head.writeUInt32LE(data.length, e + 8);
    head.writeUInt32LE(offset, e + 12);
    offset += data.length;
  });
  return Buffer.concat([head, ...frames.map((f) => f.data)]);
}
const frames = async (sizes) => Promise.all(sizes.map(async (size) => ({ size, data: await png(size <= 24 ? "small" : "transparent", size) })));

const tmp = mkdtempSync(join(tmpdir(), "ether-brand-"));
const tauri = (input, out) =>
  execFileSync("pnpm", ["--filter", "@ethereal/desktop", "exec", "tauri", "icon", input, "-o", out], { cwd: root, stdio: ["ignore", "ignore", "inherit"] });

// Desktop: everything from the transparent crystal, then the macOS .icns from the plate and the
// Windows .ico with the simplified mark at its small sizes.
writeFileSync(join(tmp, "transparent.png"), await png("transparent", 1024));
writeFileSync(join(tmp, "macos.png"), await png("macos", 1024));
rmSync(tauriIcons, { recursive: true, force: true });
tauri(join(tmp, "transparent.png"), tauriIcons);
tauri(join(tmp, "macos.png"), join(tmp, "macos"));
copyFileSync(join(tmp, "macos/icon.icns"), join(tauriIcons, "icon.icns"));
writeFileSync(join(tauriIcons, "icon.ico"), ico(await frames([16, 24, 32, 48, 64, 256])));
// No mobile app: drop the Android and iOS sets `tauri icon` adds.
rmSync(join(tauriIcons, "android"), { recursive: true, force: true });
rmSync(join(tauriIcons, "ios"), { recursive: true, force: true });
// Recompress what tauri wrote.
for (const f of ["32x32.png", "64x64.png", "128x128.png", "128x128@2x.png", "icon.png", "StoreLogo.png"].concat(
  [30, 44, 71, 89, 107, 142, 150, 284, 310].map((n) => `Square${n}x${n}Logo.png`),
)) {
  const p = join(tauriIcons, f);
  writeFileSync(p, await sharp(readFileSync(p)).png({ compressionLevel: 9, adaptiveFiltering: true }).toBuffer());
}

// Web (apps/web/public, served at the site's root).
mkdirSync(webPublic, { recursive: true });
writeFileSync(join(webPublic, "favicon.ico"), ico(await frames([16, 32, 48])));
writeFileSync(join(webPublic, "favicon.svg"), readFileSync(join(brand, "ethereal-mark-small.svg")));
writeFileSync(join(webPublic, "apple-touch-icon.png"), await png("appleTouch", 180));
writeFileSync(join(webPublic, "icon-192.png"), await png("pwa", 192));
writeFileSync(join(webPublic, "icon-512.png"), await png("pwa", 512));
writeFileSync(join(webPublic, "icon-maskable-512.png"), await png("maskable", 512));

// The contact sheet: every icon at its actual size, on the dark and the light background.
const rows = [
  ["macos (.icns)", "macos"],
  ["windows / linux", "transparent"],
  ["small mark (≤ 24 px)", "small"],
  ["pwa", "pwa"],
  ["maskable", "maskable"],
  ["apple-touch", "appleTouch"],
];
const SIZES = [16, 32, 64, 128, 256, 512, 1024];
const GAP = 24;
const rowW = SIZES.reduce((a, s) => a + s + GAP, GAP);
const rowH = 1024 + 2 * GAP;
const tiles = [];
let top = 0;
for (const [, variant] of rows) {
  for (const bg of ["#16181e", "#eceef2"]) {
    tiles.push({ input: { create: { width: rowW, height: rowH, channels: 4, background: bg } }, left: 0, top });
    let left = GAP;
    for (const s of SIZES) {
      tiles.push({ input: await png(variant, s), left, top: top + GAP + (1024 - s) });
      left += s + GAP;
    }
    top += rowH;
  }
}
await sharp({ create: { width: rowW, height: top, channels: 4, background: "#000000" } })
  .composite(tiles)
  .png()
  .toBuffer()
  .then((sheet) => sharp(sheet).resize({ width: Math.round(rowW / 2) }))
  .then((sheet) =>
    sheet // half size keeps the file small; the 16–64 px row is in contact-small.png
      .png({ palette: true, quality: 90, compressionLevel: 9 })
      .toFile(join(brand, "preview/contact-sheet.png")),
  );
// The small sizes at 1:1 (the half-size sheet blurs them).
const SMALL_SIZES = [16, 24, 32, 48, 64];
const sw = SMALL_SIZES.reduce((a, s) => a + s + 12, 12);
const sh = 64 + 24;
const small = [];
let y = 0;
for (const [, variant] of rows) {
  for (const bg of ["#16181e", "#eceef2"]) {
    small.push({ input: { create: { width: sw, height: sh, channels: 4, background: bg } }, left: 0, top: y });
    let x = 12;
    for (const s of SMALL_SIZES) {
      small.push({ input: await png(variant, s), left: x, top: y + 12 + (64 - s) });
      x += s + 12;
    }
    y += sh;
  }
}
await sharp({ create: { width: sw, height: y, channels: 4, background: "#000000" } })
  .composite(small)
  .png({ compressionLevel: 9 })
  .toFile(join(brand, "preview/contact-small.png"));

rmSync(tmp, { recursive: true, force: true });
console.log("icons written: apps/desktop/src-tauri/icons, apps/web/public, assets/brand/preview");
