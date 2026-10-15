// base-152 on the web build, through the UI, against the real engine: Ctrl+Shift+drag on an
// audio clip's body slips its content (the offset), clamped to the source. The clip's start
// and length stay; one undo per drag; a drag on the title band still moves the clip.
// Runs on the `webgl` and `canvas2d` projects (ARRANGER_SPECS).
//
// `E2E_SHOTS=<dir>` saves before/after screenshots for the PR.
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { arrangerApi, clipPart, clipPoint, dragClip, expectProjectRenderer, pixelsIn } from "./clips";
import { expandFolder, launch, openLibrary, pickOption } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const clipOf = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;
const r3 = (x: number) => +x.toFixed(3);
const bounds = async (page: Page, id: string) => {
  const c = await clipOf(page, id);
  return [r3(c.start), r3(c.length), r3(c.offset)];
};

const shot = async (page: Page, name: string, projectName: string) => {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  await page.mouse.move(0, 0);
  const box = (await page.locator(".eth-arr__scroll").boundingBox())!;
  await page.screenshot({ path: `${dir}/${projectName}-${name}.png`, clip: { x: box.x, y: box.y, width: box.width, height: 160 } });
};

const SLIP = ["Control", "Shift"] as const;

test("Ctrl+Shift+drag slips an audio clip's content, clamped to the source", async ({ page }, info) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await launch(page, `Slip ${Date.now()}`);
  await expectProjectRenderer(page, info.project.name);
  const arrangement = page.locator('[data-feature="arrangement"]');
  await pickOption(arrangement, "grid", "1/4");

  // --- A loop on an audio track ---------------------------------------------------------
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Chords Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const id = Object.keys((await doc(page)).clips)[0]!;
  const full = await clipOf(page, id);
  const start = r3(full.start);
  // The file is a whole number of beats at 120 BPM.
  const fileBeats = Math.round(full.length);
  expect(full.length).toBeCloseTo(fileBeats, 3);
  expect(full.offset).toBe(0);
  // Zoom in (Ctrl+wheel over the clip's start) until the clip is 260-450 px wide (its end left of the Inspector).
  const width = async () => (await clipPart(page, id)).width;
  for (let i = 0; i < 40 && (await width()) < 260; i++) {
    const x = (await clipPart(page, id)).x + 2;
    await page.evaluate((clientX) => {
      const el = document.querySelector<HTMLElement>(".eth-arr__scroll")!;
      const r = el.getBoundingClientRect();
      el.dispatchEvent(
        new WheelEvent("wheel", {
          deltaY: -3,
          deltaMode: WheelEvent.DOM_DELTA_LINE,
          ctrlKey: true,
          clientX,
          clientY: r.top + 20,
          bubbles: true,
          cancelable: true,
        }),
      );
    }, x);
    // The zoom applies on a later frame (slower under load): let it land before measuring,
    // or the loop overshoots the target width.
    await page.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
    await page.waitForTimeout(60);
  }
  // Settled: the width is unchanged across several polls (a lone repeat can be a frame not rendered yet).
  let lastWidth = 0;
  let same = 0;
  await expect
    .poll(
      async () => {
        const w = await width();
        same = w === lastWidth ? same + 1 : 0;
        lastWidth = w;
        return same >= 4;
      },
      { intervals: [100] },
    )
    .toBe(true);
  const px = (await width()) / full.length;

  // --- Full length: no room to slip -------------------------------------------------------
  await dragClip(page, id, -2 * px, 0, { part: "body", modifiers: [...SLIP] });
  await expect.poll(() => bounds(page, id)).toEqual([start, r3(full.length), 0]);

  // --- Shorten it to half by its right edge ---------------------------------------------
  const half = fileBeats / 2;
  await dragClip(page, id, -half * px, 0, { part: "resize-end", at: 0.5 });
  await expect.poll(() => bounds(page, id)).toEqual([start, half, 0]);
  await shot(page, "1-before-slip", info.project.name);

  // The slip cursor over the body while the modifiers are held.
  const content = page.locator(".eth-arr__content");
  const body = await clipPoint(page, id, "body");
  await page.mouse.move(body.x, body.y);
  await page.mouse.move(body.x + 2, body.y);
  for (const m of SLIP) await page.keyboard.down(m);
  await expect(content).toHaveCSS("cursor", "col-resize");
  for (const m of [...SLIP].reverse()) await page.keyboard.up(m);
  await expect(content).not.toHaveCSS("cursor", "col-resize");

  // --- Slip left by one beat: later audio shows, the offset grows ---------------------------
  const boxRect = await clipPart(page, id, "body");
  const before = await pixelsIn(page, boxRect);
  const p = await clipPoint(page, id, "body");
  await page.mouse.move(p.x, p.y);
  for (const m of SLIP) await page.keyboard.down(m);
  await page.mouse.down();
  await page.mouse.move(p.x - px, p.y, { steps: 8 });
  // Live preview: the waveform redraws mid-drag, nothing is committed yet.
  await expect.poll(async () => Buffer.compare((await pixelsIn(page, boxRect)).png, before.png) !== 0).toBe(true);
  expect(await bounds(page, id)).toEqual([start, half, 0]);
  await page.mouse.up();
  for (const m of [...SLIP].reverse()) await page.keyboard.up(m);
  await expect.poll(() => bounds(page, id)).toEqual([start, half, 1]);
  await expect.poll(async () => Buffer.compare((await pixelsIn(page, boxRect)).png, before.png) !== 0).toBe(true);
  await shot(page, "2-after-slip", info.project.name);

  // --- Clamped at both ends ---------------------------------------------------------------
  await dragClip(page, id, 3 * px, 0, { part: "body", modifiers: [...SLIP] });
  await expect.poll(() => bounds(page, id)).toEqual([start, half, 0]);
  await dragClip(page, id, -(fileBeats + 4) * px, 0, { part: "body", modifiers: [...SLIP] });
  await expect.poll(async () => (await clipOf(page, id)).offset).toBeCloseTo(full.length - half, 3);
  expect((await bounds(page, id)).slice(0, 2)).toEqual([start, half]);

  // --- One undo step per drag -------------------------------------------------------------
  await arrangement.focus();
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => bounds(page, id)).toEqual([start, half, 0]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => bounds(page, id)).toEqual([start, half, 1]);

  // --- A drag on the title band still moves the clip (offset kept) --------------------------
  await dragClip(page, id, 4 * px, 0, { part: "header" });
  await expect.poll(() => bounds(page, id)).toEqual([r3(start + 4), half, 1]);
  expect(await arrangerApi(page, "visibleClipIds")).toContain(id);

  expect(errors).toEqual([]);
});
