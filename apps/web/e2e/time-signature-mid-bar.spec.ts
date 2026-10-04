// Time-signature changes anywhere on the grid (base-108, CONTRACTS.md §11.3), on the web
// build against the real engine (WasmTransport → controller Worker).
//
// Tempo tab: double-click the signature lane mid bar 1 (beat 3, snapped to a beat), make
// it 7/8 → the arrangement ruler numbers a partial bar 1 (3 beats), bar 2 from the change
// and bar 3 one 7/8 bar later; Alt + double-click places a change off the beat grid; undo
// restores the numbering.
//
// `TIME_SIG_SHOTS=<dir>` also writes PR screenshots there. No sleeps: every step waits on
// UI or engine state (the mirror is read through `window.__ether`).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { launch, openEditor, pickOption, setNumberField } from "./ui";

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

const signatures = async (page: Page) => Object.values((await doc(page)).time_signatures).sort((a, b) => a.time - b.time);

const shots = process.env.TIME_SIG_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

/** Bar labels of the arrangement ruler: text → x (px, page coordinates). */
async function barLabels(ruler: Locator): Promise<Map<string, number>> {
  const out = new Map<string, number>();
  for (const l of await ruler.locator(".eth-ruler__label").all()) {
    const box = await l.boundingBox();
    if (box) out.set((await l.textContent())!.trim(), box.x);
  }
  return out;
}

test("time signature change mid-bar: partial bar, ruler numbering, Alt places freely", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Mid-bar ${Date.now()}`);
  await expect.poll(async () => (await signatures(page)).length).toBe(1);

  const ruler = page.locator(".eth-arr__ruler");
  // Default zoom (24 px per beat): every bar is labelled, 4/4 bars 96 px apart.
  await expect.poll(async () => {
    const l = await barLabels(ruler);
    return [l.has("1"), l.has("2"), l.has("3")];
  }).toEqual([true, true, true]);
  const before = await barLabels(ruler);
  const pxPerBeat = (before.get("2")! - before.get("1")!) / 4;
  expect(pxPerBeat).toBeGreaterThan(0);

  // --- A change at beat 3 (mid bar 1, snapped to a 4/4 beat), set to 7/8 ------------------
  await openEditor(page, "Tempo");
  const lane = page.getByTestId("signature-lane");
  await expect(lane).toBeVisible();
  // The tempo editor draws 12 px per beat from beat 0: x = 37 → beat 3.08 → beat 3.
  await lane.dblclick({ position: { x: 12 * 3 + 1, y: 6 } });
  await expect.poll(async () => (await signatures(page)).length).toBe(2);
  const mid = (await signatures(page))[1]!;
  expect(mid.time).toBe(3);
  await setNumberField(page.getByRole("spinbutton", { name: "Beats per bar" }), 7);
  await expect.poll(async () => (await doc(page)).time_signatures[mid.id]?.signature.numerator).toBe(7);
  await pickOption(page, "Beat unit", { value: "8" });
  await expect.poll(async () => (await doc(page)).time_signatures[mid.id]?.signature).toEqual({ numerator: 7, denominator: 8 });

  // Ruler: bar 1 is a partial bar (3 beats), bar 2 starts at the change, bar 3 one 7/8 bar
  // (3.5 beats) later, at beat 6.5.
  const x0 = before.get("1")!;
  await expect
    .poll(async () => {
      const l = await barLabels(ruler);
      return [
        Math.round((l.get("2")! - x0) / pxPerBeat * 100) / 100,
        Math.round((l.get("3")! - x0) / pxPerBeat * 100) / 100,
      ];
    })
    .toEqual([3, 6.5]);
  // The ruler's signature marker sits on the bar-2 label.
  const marker = ruler.locator(`[data-signature="${mid.id}"]`);
  await expect(marker).toBeVisible();
  expect(Math.abs((await marker.boundingBox())!.x - (await barLabels(ruler)).get("2")!)).toBeLessThan(6);
  if (shots) await page.screenshot({ path: `${shots}/01-mid-bar-7-8.png` });

  // --- Alt + double-click: no snapping (a change off the beat grid) ------------------------
  await lane.dblclick({ position: { x: Math.round(12 * 9.3), y: 6 }, modifiers: ["Alt"] });
  await expect.poll(async () => (await signatures(page)).length).toBe(3);
  const free = (await signatures(page))[2]!;
  expect(free.time).toBeGreaterThan(9.1);
  expect(free.time).toBeLessThan(9.5);
  expect(Math.abs(free.time * 2 - Math.round(free.time * 2))).toBeGreaterThan(0.01); // off the 7/8 grid
  if (shots) await page.screenshot({ path: `${shots}/02-free-placement.png` });

  // --- Undo: ordinary edits, the numbering follows ----------------------------------------
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await signatures(page)).length).toBe(2);
  // Undo the 7/8 edits and the add (the signature fields may take one step each).
  await expect
    .poll(async () => {
      if ((await signatures(page)).length > 1) await page.getByRole("button", { name: "Undo" }).click();
      return (await signatures(page)).length;
    })
    .toBe(1);
  await expect
    .poll(async () => {
      const l = await barLabels(ruler);
      return Math.round((l.get("2")! - x0) / pxPerBeat * 100) / 100;
    })
    .toBe(4);

  expect(errors).toEqual([]);
});
