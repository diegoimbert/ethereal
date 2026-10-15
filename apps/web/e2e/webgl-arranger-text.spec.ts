// The arranger's text against the DOM's, pixel for pixel, on the backend the project runs
// (`webgl`, `canvas2d`): clip names (the title font, 10 px / 500) in both themes (light ink
// on dark bands and dark ink on light ones) and ellipsized names.
//
// The reference is DOM text the test lays over the clip itself: the name in a
// `.eth-clip__name` element (the font, features and ellipsis the canvas text copies) in the
// title band's ink, on the band's color, `text-overflow: ellipsis` within the name's room
// (`nameRect`); the chip as a span styled like it. Crops of the device pixels, aligned on
// the text's ink, must agree within 8/255 on 99 % of the pixels, and of the text's own
// pixels ("ink": where either differs from the band). Ellipsized names agreeing means the
// same cut as `text-overflow: ellipsis`.
//
// Measured (Chromium, Linux): the SDF glyphs this replaced agreed on 78-84 % of the pixels
// (5-29 % of the ink); the browser-rasterized sprites agree on 100 %, within 2/255.
//
// `ETHER_TEXT_SHOTS=<dir>`: also writes each crop (DOM and canvas) there as PNG.
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { arrangement, arrangerApi, clipPart, pixelsIn, projectBackend, type Rect } from "./clips";
import { installProbe, send, setViewport, ulid } from "./perfProject";
import { launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

/** Names, the clips' lengths (beats, at 24 px per beat) and colors (bands light and dark), on two tracks. */
const CLIPS: ReadonlyArray<{
  name: string;
  length: number;
  color: number | null;
  track?: 2;
}> = [
  { name: "Bass Line 2", length: 6, color: null },
  { name: "Synth Pad (copy)", length: 6, color: 0xf2d16b },
  { name: "Vocals — Take 0123", length: 7, color: 0x2b4c7e },
  { name: "Kick & Snare fx", length: 6, color: 0x9ad0f5 },
  { name: "Ünïcødé àçcents gjpqy", length: 8, color: 0x3d3d3d },
  // Ellipsized (text-overflow: ellipsis):
  {
    name: "A very long clip name that the ellipsis truncates",
    length: 4,
    color: null,
    track: 2,
  },
  {
    name: "Another long name, cut short WAVE 12",
    length: 5,
    color: 0xf2d16b,
    track: 2,
  },
  {
    name: "Overdub   spaces   before the cut",
    length: 4,
    color: 0x2b4c7e,
    track: 2,
  },
  {
    name: "Lead synth (bounced) — final mix",
    length: 6,
    color: 0x9ad0f5,
    track: 2,
  },
];
type Crop = Awaited<ReturnType<typeof pixelsIn>>;

/** Per-pixel agreement of two crops: share of pixels within 8/255, the worst channel diff, ink share. */
interface Agreement {
  within8: number;
  max: number;
  /** Among "ink" pixels (where either crop differs from the band by more than 8). */
  inkWithin8: number;
  ink: number;
  /** The reference's offset applied (device px) to align the ink. */
  shift: [number, number];
}

/** The crop's most common color (the band). */
function bandOf(a: Crop): [number, number, number] {
  const counts = new Map<number, number>();
  for (let i = 0; i < a.data.length; i += 4) {
    const k = (a.data[i]! << 16) | (a.data[i + 1]! << 8) | a.data[i + 2]!;
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  const key = [...counts].sort((x, y) => y[1] - x[1])[0]![0];
  return [(key >> 16) & 255, (key >> 8) & 255, key & 255];
}

/** Device pixels the reference may be moved by to meet the drawn text (each way). */
const MAX_SHIFT = 2;

/**
 * `drawn` against the DOM reference `ref` (same crop) on `band`, the reference moved by the
 * shift (within `MAX_SHIFT` device px) where they agree best: the DOM's layout boxes sit
 * where the canvas's do within a device pixel or so; what is compared is the text itself.
 * Pixels shifted in from outside count as band.
 */
function compare(ref: Crop, drawn: Crop, band: readonly number[]): Agreement {
  let best: Agreement | null = null;
  for (let dy = -MAX_SHIFT; dy <= MAX_SHIFT; dy++) {
    for (let dx = -MAX_SHIFT; dx <= MAX_SHIFT; dx++) {
      const a = compareAt(ref, drawn, band, [dx, dy]);
      if (!best || a.within8 + a.inkWithin8 > best.within8 + best.inkWithin8) best = a;
    }
  }
  return best!;
}

function compareAt(ref: Crop, drawn: Crop, band: readonly number[], shift: [number, number]): Agreement {
  let ok = 0;
  let max = 0;
  let ink = 0;
  let inkOk = 0;
  const n = drawn.width * drawn.height;
  for (let y = 0; y < drawn.height; y++) {
    for (let x = 0; x < drawn.width; x++) {
      const i = (y * drawn.width + x) * 4;
      const rx = x - shift[0];
      const ry = y - shift[1];
      const inside = rx >= 0 && ry >= 0 && rx < ref.width && ry < ref.height;
      const j = (ry * ref.width + rx) * 4;
      let d = 0;
      let fromBand = 0;
      for (let k = 0; k < 3; k++) {
        const r = inside ? ref.data[j + k]! : band[k]!;
        d = Math.max(d, Math.abs(r - drawn.data[i + k]!));
        fromBand = Math.max(fromBand, Math.abs(r - band[k]!), Math.abs(drawn.data[i + k]! - band[k]!));
      }
      max = Math.max(max, d);
      if (d <= 8) ok++;
      if (fromBand > 8) {
        ink++;
        if (d <= 8) inkOk++;
      }
    }
  }
  return {
    within8: ok / n,
    max,
    inkWithin8: ink ? inkOk / ink : 1,
    ink,
    shift,
  };
}

/** DOM text of a reference: its box (client px; a chip's `x` is its right edge) and style. */
interface RefText {
  box: Rect;
  content: string;
  ink: string;
  chip?: { color: string; padX: string; radius: string };
}

/**
 * Lays DOM text over `crop` (client px) on the band's color, screenshots the same crop, and
 * removes it.
 */
async function domReference(page: Page, crop: Rect, band: readonly number[], texts: RefText[]): Promise<Crop> {
  await page.evaluate(
    ([crop, band, texts]) => {
      const host = document.createElement("div");
      host.id = "text-reference";
      Object.assign(host.style, {
        position: "fixed",
        left: `${crop.x}px`,
        top: `${crop.y}px`,
        width: `${crop.width}px`,
        height: `${crop.height}px`,
        overflow: "hidden",
        zIndex: "100000",
        background: `rgb(${band[0]}, ${band[1]}, ${band[2]})`,
        pointerEvents: "none",
      });
      for (const text of texts) {
        // The name element of the clip title (its font, weight and ellipsis), on the band's line box.
        const name = document.createElement("span");
        name.className = "eth-clip__name eth-user-text";
        name.textContent = text.content;
        Object.assign(name.style, {
          position: "absolute",
          // A chip is placed by its right edge.
          ...(text.chip ? { right: `${crop.x + crop.width - text.box.x}px` } : { left: `${text.box.x - crop.x}px` }),
          top: `${text.box.y - crop.y}px`,
          height: `${text.box.height}px`,
          lineHeight: `${text.box.height}px`,
          fontSize: "var(--eth-fs-xs)",
          whiteSpace: "nowrap",
          color: text.ink,
          padding: text.chip ? `0 ${text.chip.padX}` : "0",
          borderRadius: text.chip ? text.chip.radius : "0",
          background: text.chip ? text.chip.color : "transparent",
          display: "block",
          ...(text.chip
            ? {}
            : {
                width: `${text.box.width}px`,
                overflow: "hidden",
                textOverflow: "ellipsis",
              }),
        });
        host.append(name);
      }
      document.body.append(host);
    },
    [crop, band, texts] as const,
  );
  const shot = await pixelsIn(page, crop, "device");
  await page.evaluate(() => document.getElementById("text-reference")?.remove());
  return shot;
}

async function project(page: Page): Promise<string[]> {
  await installProbe(page);
  const tracks = [ulid(8, 1), ulid(8, 2)];
  for (const [i, id] of tracks.entries()) {
    await send(page, {
      domain: "Track",
      command: {
        type: "Create",
        id,
        kind: "Midi",
        name: `Text ${i + 1}`,
        color: null,
        parent: null,
        before: null,
      },
    });
  }
  const ids: string[] = [];
  const starts = [0, 0];
  for (const [i, c] of CLIPS.entries()) {
    const id = ulid(2, i + 1);
    const t = c.track === 2 ? 1 : 0;
    await send(page, {
      domain: "Clip",
      command: {
        type: "CreateMidi",
        id,
        track: tracks[t]!,
        start: starts[t]!,
        length: c.length,
        name: c.name,
      },
    });
    if (c.color !== null)
      await send(page, {
        domain: "Clip",
        command: { type: "SetColor", id, color: c.color },
      });
    ids.push(id);
    starts[t] = starts[t]! + c.length + 1;
  }
  await setViewport(page, 24, 0);
  return ids;
}

const shotsDir = process.env.ETHER_TEXT_SHOTS;

/**
 * The header column's soft shadow falls on the first pixels of the lanes (a decorative overlay
 * the DOM reference, painted over a flat band color, doesn't have): off while comparing text.
 */
async function noHeaderShade(page: Page): Promise<void> {
  await page.addStyleTag({ content: ".eth-arr__header-shade { box-shadow: none !important; }" });
}

function save(name: string, ref: Crop, drawn: Crop): void {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  writeFileSync(join(shotsDir, `${name}-dom.png`), ref.png);
  writeFileSync(join(shotsDir, `${name}-canvas.png`), drawn.png);
}

for (const dpr of [1, 2]) {
  test.describe(`at ${dpr}x`, () => {
    test.use({ deviceScaleFactor: dpr });

    test(`clip names match the DOM's pixels (${dpr}x)`, async ({ page }) => {
      test.setTimeout(180_000);
      await launch(page, `Arranger text pixels ${Date.now()}`);
      await noHeaderShade(page);
      const backend = projectBackend(test.info().project.name);
      await expect.poll(async () => (await arrangement(page)).renderer).toBe(backend);
      const ids = await project(page);
      await page.mouse.move(5, 5);
      const report: string[] = [];
      let worstAll = 1;
      let worstInk = 1;
      for (const theme of ["dark", "light"] as const) {
        await page.evaluate((t) => {
          document.documentElement.dataset.theme = t;
        }, theme);
        await page.waitForTimeout(400);
        let all = 0;
        let ok = 0;
        let inkAll = 0;
        let inkOk = 0;
        let max = 0;
        for (const [i, id] of ids.entries()) {
          const r = await clipPart(page, id, "header");
          // Inside the band's rounded corners and the clip's edges.
          const crop = {
            x: r.x + 3,
            y: r.y + 1,
            width: r.width - 6,
            height: r.height - 1,
          };
          const drawn = await pixelsIn(page, crop, "device");
          const band = bandOf(drawn);
          const box = (await arrangerApi(page, "nameRect", id))!;
          const ref = await domReference(page, crop, band, [
            {
              box,
              content: CLIPS[i]!.name,
              ink: "var(--eth-color-text-inverse)",
            },
          ]);
          const a = compare(ref, drawn, band);
          const n = drawn.width * drawn.height;
          all += n;
          ok += a.within8 * n;
          inkAll += a.ink;
          inkOk += a.inkWithin8 * a.ink;
          max = Math.max(max, a.max);
          worstAll = Math.min(worstAll, a.within8);
          worstInk = Math.min(worstInk, a.inkWithin8);
          report.push(
            `${backend} ${theme} ${dpr}x "${CLIPS[i]!.name}": ${(a.within8 * 100).toFixed(2)}% within 8/255 (ink pixels ${(a.inkWithin8 * 100).toFixed(1)}% of ${a.ink}), max ${a.max}, shift ${a.shift}`,
          );
          save(`${backend}-${theme}-${dpr}x-${i}`, ref, drawn);
        }
        report.push(
          `${backend} ${theme} ${dpr}x ALL: ${((ok / all) * 100).toFixed(2)}% within 8/255; ink pixels ${((inkOk / inkAll) * 100).toFixed(2)}% of ${inkAll}; max ${max}`,
        );
      }
      console.log(report.join("\n"));
      expect(worstAll, report.join("\n")).toBeGreaterThanOrEqual(0.99);
      expect(worstInk, report.join("\n")).toBeGreaterThanOrEqual(0.99);
    });
  });
}
