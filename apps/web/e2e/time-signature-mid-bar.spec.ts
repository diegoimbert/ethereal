// Time-signature changes anywhere on the grid (base-108, CONTRACTS.md §11.3), on the web
// build against the real engine (WasmTransport → controller Worker).
//
// The arrangement ruler's context menu adds a change mid bar 1 (beat 3, snapped to a beat);
// its chip's menu (in the marker lane) makes it 7/8 → the ruler numbers a partial bar 1 (3
// beats), bar 2 from the change and bar 3 one 7/8 bar later; dragging the chip snaps to the
// grid (Alt: free); Alt + right-click places a change off the beat grid; undo restores the
// numbering. (The Tempo tab, where this was done before, is gone: d5d2e4f9 / e6d68591.)
//
// `TIME_SIG_SHOTS=<dir>` also writes PR screenshots there. No sleeps: every step waits on
// UI or engine state (the mirror is read through `window.__ether`).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { launch } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const signatures = async (page: Page) => Object.values((await doc(page)).time_signatures).sort((a, b) => a.time - b.time);

const shots = process.env.TIME_SIG_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

/** Bar labels of the arrangement ruler: text → x (px, page coordinates). The labels are
 *  drawn on the ruler's marks canvas, which lists them in `data-labels` (`[text, x]`). */
async function barLabels(ruler: Locator): Promise<Map<string, number>> {
  const canvas = ruler.getByTestId("ruler-marks");
  const box = await canvas.boundingBox();
  const labels = JSON.parse((await canvas.getAttribute("data-labels")) ?? "[]") as Array<[string, number]>;
  return new Map(labels.map(([text, x]) => [text, (box?.x ?? 0) + x]));
}

test("time signature change mid-bar: partial bar, ruler numbering, Alt places freely", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Mid-bar ${Date.now()}`);
  await expect.poll(async () => (await signatures(page)).length).toBe(1);

  const ruler = page.locator(".eth-arr__ruler");
  // Default zoom (24 px per beat): every bar is labelled, 4/4 bars 96 px apart.
  await expect
    .poll(async () => {
      const l = await barLabels(ruler);
      return [l.has("1"), l.has("2"), l.has("3")];
    })
    .toEqual([true, true, true]);
  const before = await barLabels(ruler);
  const pxPerBeat = (before.get("2")! - before.get("1")!) / 4;
  expect(pxPerBeat).toBeGreaterThan(0);

  // --- A change at beat 3 (mid bar 1, snapped to a 4/4 beat), set to 7/8 ------------------
  // Right-click the ruler at `beat` (labels sit 3 px right of their bar line) and add a
  // signature change there.
  const rulerBox = (await ruler.boundingBox())!;
  const beat0 = before.get("1")! - 3;
  const addSignatureAt = async (beat: number, modifiers: "Alt"[] = []) => {
    await ruler.click({ button: "right", modifiers, position: { x: beat0 - rulerBox.x + beat * pxPerBeat, y: rulerBox.height / 2 } });
    await page.getByRole("menuitem", { name: "add time signature change here" }).click();
  };
  await addSignatureAt(3.2);
  await expect.poll(async () => (await signatures(page)).length).toBe(2);
  const mid = (await signatures(page))[1]!;
  expect(mid.time).toBe(3);
  await page.getByTestId("marker-lane").locator(`[data-signature="${mid.id}"]`).click({ button: "right" });
  await page.getByRole("menuitem", { name: "7/8", exact: true }).click();
  await expect.poll(async () => (await doc(page)).time_signatures[mid.id]?.signature).toEqual({ numerator: 7, denominator: 8 });

  // Ruler: bar 1 is a partial bar (3 beats), bar 2 starts at the change, bar 3 one 7/8 bar
  // (3.5 beats) later, at beat 6.5.
  const x0 = before.get("1")!;
  await expect
    .poll(async () => {
      const l = await barLabels(ruler);
      return [Math.round(((l.get("2")! - x0) / pxPerBeat) * 100) / 100, Math.round(((l.get("3")! - x0) / pxPerBeat) * 100) / 100];
    })
    .toEqual([3, 6.5]);
  // The ruler's signature marker sits on the bar-2 label.
  // The signature chip lives in the marker lane (#401), under the ruler's numbers.
  const marker = page.getByTestId("marker-lane").locator(`[data-signature="${mid.id}"]`);
  await expect(marker).toBeVisible();
  expect(Math.abs((await marker.boundingBox())!.x - (await barLabels(ruler)).get("2")!)).toBeLessThan(6);
  if (shots) await page.screenshot({ path: `${shots}/01-mid-bar-7-8.png` });

  // --- Drag the chip: on the arrangement grid by default, free with Alt -------------------
  // The grid applies in the meter before the change (4/4 from beat 0): the default adaptive
  // grid is a beat or finer, so the change lands on a beat subdivision, never in between.
  const dragChip = async (beats: number, alt = false) => {
    const box = (await marker.boundingBox())!;
    const x = box.x + box.width / 2;
    const y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    if (alt) await page.keyboard.down("Alt");
    for (let i = 1; i <= 4; i++) await page.mouse.move(x + (beats * pxPerBeat * i) / 4, y);
    await page.mouse.up();
    if (alt) await page.keyboard.up("Alt");
  };
  await dragChip(1.37);
  await expect.poll(async () => (await doc(page)).time_signatures[mid.id]!.time).toBeGreaterThan(3.9);
  const snapped = (await doc(page)).time_signatures[mid.id]!.time;
  expect(snapped).toBeLessThan(4.6);
  expect(Math.abs(snapped * 4 - Math.round(snapped * 4))).toBeLessThan(1e-6); // on the grid
  await dragChip(0.13, true);
  await expect.poll(async () => (await doc(page)).time_signatures[mid.id]!.time).not.toBe(snapped);
  const dragged = (await doc(page)).time_signatures[mid.id]!.time;
  expect(Math.abs(dragged * 4 - Math.round(dragged * 4))).toBeGreaterThan(0.01); // off the grid
  // Back to beat 3 (undo both drags) for the rest of the spec.
  for (const want of [snapped, 3]) {
    await page.getByRole("button", { name: "undo" }).click();
    await expect.poll(async () => (await doc(page)).time_signatures[mid.id]!.time).toBe(want);
  }

  // --- Alt + right-click: no snapping (a change off the beat grid) ------------------------
  await addSignatureAt(9.3, ["Alt"]);
  await expect.poll(async () => (await signatures(page)).length).toBe(3);
  const free = (await signatures(page))[2]!;
  expect(free.time).toBeGreaterThan(9.1);
  expect(free.time).toBeLessThan(9.5);
  expect(Math.abs(free.time * 2 - Math.round(free.time * 2))).toBeGreaterThan(0.01); // off the 7/8 grid
  if (shots) await page.screenshot({ path: `${shots}/02-free-placement.png` });

  // --- Undo: ordinary edits, the numbering follows ----------------------------------------
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(async () => (await signatures(page)).length).toBe(2);
  // Undo the 7/8 edits and the add (the signature fields may take one step each).
  await expect
    .poll(async () => {
      if ((await signatures(page)).length > 1) await page.getByRole("button", { name: "undo" }).click();
      return (await signatures(page)).length;
    })
    .toBe(1);
  await expect
    .poll(async () => {
      const l = await barLabels(ruler);
      return Math.round(((l.get("2")! - x0) / pxPerBeat) * 100) / 100;
    })
    .toBe(4);

  expect(errors).toEqual([]);
});
