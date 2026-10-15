// Screenshots of the in-clip editing overlays (fades, crossfade, clip gain, reversed badge)
// on WebGL and on the Canvas2D fallback, dark and light (PR material). `E2E_SHOTS=<dir>`
// saves them; without it the spec checks that both backends draw the same overlays (the
// test API's `clipDecor`) with close colors.
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { clipDecor, clipPart, pixelAt, setArrangerRenderer } from "./clips";
import { doc, importBassLoop, installProbe, send, setViewport, ulid } from "./perfProject";
import { createTrack, launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const dir = process.env.E2E_SHOTS;

async function shoot(page: Page, name: string, clips: string[]): Promise<void> {
  // Peaks arrive and the lanes settle.
  await page.waitForTimeout(800);
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  const boxes = await Promise.all(clips.map((c) => clipPart(page, c)));
  const x0 = Math.min(...boxes.map((b) => b.x)) - 16;
  const x1 = Math.max(...boxes.map((b) => b.x + b.width)) + 16;
  const y0 = Math.min(...boxes.map((b) => b.y)) - 8;
  const y1 = Math.max(...boxes.map((b) => b.y + b.height)) + 8;
  await page.screenshot({ path: join(dir, `${name}.png`), clip: { x: x0, y: y0, width: x1 - x0, height: y1 - y0 } });
}

test("WebGL vs Canvas2D: fades, crossfade, gain, reversed", async ({ page }) => {
  test.skip(test.info().project.name === "canvas2d", "compares both backends in one run");
  test.setTimeout(180_000);
  await launch(page, `Clip edit shots ${Date.now()}`);
  await installProbe(page);
  const track = await createTrack(page, "Audio");
  await importBassLoop(page, track.id);
  await page.getByRole("button", { name: "close the library panel" }).click();
  const inspector = page.getByRole("button", { name: "collapse the inspector panel" });
  if (await inspector.isVisible()) await inspector.click();
  const src = Object.values((await doc(page)).clips)[0]!;
  const L = src.length;
  const [c0, c1, c2, c3] = [src.id, ulid(9, 11), ulid(9, 12), ulid(9, 13)];
  const clip = (domain: "Clip", command: object) => send(page, { domain, command } as Parameters<typeof send>[1]);
  // Fades (a linear fade-in, a bent fade-out), two touching clips crossfaded, a reversed
  // clip with +12 dB (marked past full scale).
  await clip("Clip", { type: "SetFades", id: c0, fade_in: L / 4, fade_out: L / 3 });
  await clip("Clip", { type: "SetFadeCurves", id: c0, fade_in: null, fade_out: { type: "Curve", tension: 0.6 } });
  await clip("Clip", { type: "Duplicate", id: c0, new_id: c1, start: L + 1 });
  await clip("Clip", { type: "SetFades", id: c1, fade_in: 0, fade_out: 0 });
  // Split in the middle: both halves have material across the boundary to crossfade with.
  await clip("Clip", { type: "Split", id: c1, at: 1.5 * L + 1, new_id: c2 });
  await clip("Clip", { type: "Crossfade", first: c1, second: c2, length: L / 4, curve: { type: "EqualPower" } });
  await clip("Clip", { type: "Duplicate", id: c0, new_id: c3, start: 2 * L + 3 });
  await clip("Clip", { type: "SetFades", id: c3, fade_in: 0, fade_out: 0 });
  await clip("Clip", { type: "SetReversed", id: c3, reversed: true });
  await clip("Clip", { type: "SetGain", id: c3, gain: 12 });
  const all = [c0, c1, c2, c3];

  const decor: Record<string, unknown> = {};
  const pixels: Record<string, Array<[number, number, number]>> = {};
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    for (const renderer of ["webgl", "canvas2d"] as const) {
      await setArrangerRenderer(page, renderer);
      const width = await page.locator(".eth-arr__scroll").evaluate((el) => el.clientWidth);
      await setViewport(page, Math.floor((width - 400) / (3 * L + 4)), -0.5);
      // c0 selected (its handles show), the pointer over c2's title (hovered: its handles).
      const h0 = await clipPart(page, c0, "header");
      await page.mouse.click(h0.x + h0.width / 2, h0.y + h0.height / 2);
      const h2 = await clipPart(page, c2, "header");
      await page.mouse.move(h2.x + h2.width / 2, h2.y + h2.height / 2);
      await expect.poll(async () => (await clipDecor(page, c2))?.crossfadeIn).toBe(true);
      await expect.poll(async () => (await clipDecor(page, c3))?.badges).toEqual(["reversed"]);
      decor[renderer] = await Promise.all(all.map((c) => clipDecor(page, c)));
      await shoot(page, `clipedit-${theme}-${renderer}`, all);
      // Pixel parity probes: the fade-in shade of c0 (above its curve), the crossfade tint
      // (top middle of the overlap, above both curves).
      const b0 = await clipPart(page, c0, "body");
      const ppb = b0.width / L;
      const fin = ppb * (L / 4);
      const b2 = await clipPart(page, c2, "body");
      // c2 starts L/8 before the split: the overlap is its first L/4 beats.
      const half = ppb * (L / 8);
      pixels[renderer] = [await pixelAt(page, b0.x + fin * 0.75, b0.y + 2), await pixelAt(page, b2.x + half, b2.y + 2)];
    }
    // The same overlays on both backends.
    expect(decor.canvas2d).toEqual(decor.webgl);
    pixels.canvas2d!.forEach((c, i) => c.forEach((v, k) => expect(Math.abs(v - pixels.webgl![i]![k]!), `probe ${i} (${theme})`).toBeLessThanOrEqual(12)));
  }
});
