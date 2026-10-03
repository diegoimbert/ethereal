// Note stretch (base-109, owner request, Ableton-style) on the web build, through the UI,
// against the real engine (WasmTransport → controller Worker → AudioWorklet). The owner's
// steps: a MIDI clip with notes at beats 0 and 2; select the section 0..4 on the strip under
// the ruler; a bar appears on the ruler over the selection; drag its right edge from 4 to 8:
// starts, durations and the gap double about the left edge (notes at 0 and 4, twice as long)
// and the clip grows to 8. One ⌘Z undoes the whole drag.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots (dark).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const r3 = (x: number) => +x.toFixed(3);

/** A clip's notes as sorted [start, duration] (rounded). */
const placed = async (page: Page, clip: string): Promise<number[][]> =>
  Object.values((await doc(page)).notes)
    .filter((n) => n.clip === clip)
    .map((n) => [r3(n.start), r3(n.duration)])
    .sort((a, b) => a[0]! - b[0]!);

const clipLength = async (page: Page, clip: string) => r3((await doc(page)).clips[clip]!.length);

const shot = async (target: Locator, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await target.screenshot({ path: `${dir}/${name}.png` });
};

/** The piano roll's px per beat (from the 4-beat clip's start and end markers). */
async function pxPerBeat(page: Page) {
  const left = (sel: string) => page.locator(sel).evaluate((el) => parseFloat((el as HTMLElement).style.left));
  return ((await left('[data-testid="piano-roll-clip-end"]')) - (await left(".eth-pr__start"))) / 4;
}

/** Client x of a content beat in the open piano roll. */
async function beatX(page: Page) {
  const gridBox = (await page.getByTestId("piano-roll-grid").boundingBox())!;
  const x0 = await page.locator(".eth-pr__start").evaluate((el) => parseFloat((el as HTMLElement).style.left));
  const px = await pxPerBeat(page);
  return (beat: number) => gridBox.x + x0 + beat * px;
}

/** Zooms out (ctrl + wheel) until `beats` fit in the left half of the visible grid. */
async function fitRoll(page: Page, beats: number) {
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const room = (body.width - 64) / 2;
  await page.mouse.move(body.x + 70, body.y + body.height / 2);
  for (let i = 0; i < 20 && (await pxPerBeat(page)) * beats > room; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, 200);
    await page.keyboard.up("Control");
  }
  expect((await pxPerBeat(page)) * beats).toBeLessThanOrEqual(room);
}

test("note stretch: drag the ruler bar's right edge to double the selected section", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Note stretch ${Date.now()}`);

  // --- A one-bar MIDI clip with notes at beats 0 and 2 ---------------------------------------
  const t = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === t.id).length).toBe(1);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!.id;
  await openClip(page, clip);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await fitRoll(page, 12);
  let x = await beatX(page);
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const y = body.y + body.height / 2;
  for (const beat of [0, 2]) {
    const before = (await placed(page, clip)).length;
    await page.mouse.dblclick(x(beat + 0.1), y);
    await expect.poll(async () => (await placed(page, clip)).length).toBe(before + 1);
  }
  const drawn = await placed(page, clip);
  expect(drawn.map(([s]) => s)).toEqual([0, 2]);
  const d = drawn[0]![1]!;
  expect(drawn[1]![1]).toBe(d);

  // A click on empty space drops the selection and places the insert marker (a zero-length
  // selection): no bar.
  await page.mouse.click(x(3.5), y - 60);
  await expect(page.getByTestId("piano-roll-marker")).toBeVisible();
  await expect(page.getByTestId("piano-roll-stretch")).toHaveCount(0);

  // --- Select the section 0..4 on the strip under the ruler ----------------------------------
  x = await beatX(page);
  const strip = (await page.getByTestId("piano-roll-loopbar").boundingBox())!;
  const sy = strip.y + strip.height / 2;
  await page.mouse.move(x(0) + 1, sy);
  await page.mouse.down();
  await page.mouse.move(x(2), sy, { steps: 3 });
  await page.mouse.move(x(4), sy, { steps: 3 });
  await page.mouse.up();
  await expect(page.getByTestId("piano-roll-section")).toBeVisible();

  // --- The bar spans 0..4; its right edge shows a resize cursor ------------------------------
  const bar = page.getByTestId("piano-roll-stretch");
  await expect(bar).toBeVisible();
  const box = (await bar.boundingBox())!;
  expect(Math.abs(box.x - x(0))).toBeLessThan(2);
  expect(Math.abs(box.x + box.width - x(4))).toBeLessThan(2);
  const by = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width - 2, by);
  await expect.poll(() => bar.evaluate((el) => (el as HTMLElement).style.cursor)).toBe("ew-resize");
  const roll = page.getByTestId("piano-roll");
  await shot(roll, "note-stretch-bar");

  // --- Drag the right edge from 4 to 8: everything doubles about 0, the clip grows -----------
  await page.mouse.down();
  await page.mouse.move(x(6) - 2, by, { steps: 4 });
  await page.mouse.move(x(8) - 2, by, { steps: 4 });
  await expect.poll(() => placed(page, clip)).toEqual([
    [0, r3(2 * d)],
    [4, r3(2 * d)],
  ]);
  await shot(roll, "note-stretch-dragging");
  await page.mouse.up();
  await expect.poll(() => clipLength(page, clip)).toBe(8);
  await shot(roll, "note-stretch-after");

  // --- One undo step for the whole drag -------------------------------------------------------
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => placed(page, clip)).toEqual(drawn);
  await expect.poll(() => clipLength(page, clip)).toBe(4);

  expect(errors).toEqual([]);
});
