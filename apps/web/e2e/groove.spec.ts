// Groove on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → clip → draw notes in the piano roll → Quantize… popover (1/8, 100%, swing
// 100%): starts land on eighths, odd ones delayed by 1/6 beat → right-click a note →
// Humanize (the clicked note moves) → the clip inspector's groove: project swing 50% on 1/8.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip } from "./clips";
import { clickPianoRollNote, launch, newProject, pianoRollNotes } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const notes = async (page: Page): Promise<Note[]> => Object.values((await doc(page)).notes).sort((a, b) => a.pitch - b.pitch || a.start - b.start);

/** Start on an eighth grid, off-beats delayed by `0.5 / 3` (full quantize swing). */
function onSwungEighths(start: number): boolean {
  const k = Math.round(start / 0.5);
  const target = k * 0.5 + (Math.abs(k % 2) === 1 ? 0.5 / 3 : 0);
  return Math.abs(start - target) < 1e-6;
}

test("groove: quantize with swing, humanize, project swing", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await newProject(page, `Groove ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);

  // --- MIDI clip in the piano roll, with notes off the grid ---------------------------------
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create MIDI track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Midi")).toBe(true);
  const midi = Object.values((await doc(page)).tracks).find((t) => t.kind === "Midi")!;
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  // Grid off, so drawn notes start wherever we click.
  await page.getByTestId("piano-roll").getByRole("combobox", { name: "grid", exact: true }).click();
  await page.getByRole("option", { name: "off", exact: true }).click();
  const gridBox = (await grid.boundingBox())!;
  const endBox = (await page.getByTestId("piano-roll-clip-end").boundingBox())!;
  const clipWidth = endBox.x - gridBox.x;
  for (const [fx, fy] of [
    [0.03, 0.4],
    [0.14, 0.45],
    [0.4, 0.5],
  ] as const) {
    await grid.dblclick({ position: { x: clipWidth * fx + 2, y: gridBox.height * fy } });
  }
  await expect.poll(async () => (await notes(page)).length).toBe(3);

  // --- Quantize… popover: 1/8, strength 100%, swing 100% (all notes selected) ----------------
  await page.getByTestId("piano-roll").press("ControlOrMeta+a");
  await page.getByRole("button", { name: "quantize…" }).click();
  await page.getByRole("combobox", { name: "quantize grid" }).click();
  await page.getByRole("option", { name: "1/8", exact: true }).click();
  // (Exact: the clip inspector's groove has "project swing".)
  const swing = page.getByRole("spinbutton", { name: "swing", exact: true });
  await swing.fill("100");
  await swing.press("Enter");
  await page.getByRole("button", { name: "quantize 3 selected" }).click();
  await expect.poll(async () => (await notes(page)).every((n) => onSwungEighths(n.start))).toBe(true);

  // --- Right-click a note → Humanize: only that note changes ---------------------------------
  const before = await notes(page);
  await page.getByTestId("piano-roll").press("Escape");
  // A note in view: the grid is the whole keyboard's height, scrolled in the drawer.
  const view = (await page.locator(".eth-pr__body").boundingBox())!;
  const shown = (await pianoRollNotes(grid)).find((n) => n.y >= view.y && n.y + n.height <= view.y + view.height);
  expect(shown).toBeDefined();
  await clickPianoRollNote(page, shown!, { button: "right" });
  await page.getByRole("menuitem", { name: /humanize/ }).click();
  await expect
    .poll(async () => {
      const after = await notes(page);
      return after.filter((n, i) => n.start !== before[i]!.start || n.velocity !== before[i]!.velocity).length;
    })
    .toBe(1);

  // --- Project swing: the clip inspector's groove row (the Groove tab is gone, e6d68591) ----
  const amount = page.getByRole("spinbutton", { name: "project swing" });
  await amount.fill("50");
  await amount.press("Enter");
  await page.getByRole("combobox", { name: "swing grid" }).click();
  await page.getByRole("option", { name: "1/8", exact: true }).click();
  await expect.poll(async () => (await doc(page)).settings.swing).toBeCloseTo(0.5, 5);
  await expect.poll(async () => (await doc(page)).settings.swing_grid).toBe(0.5);

  expect(errors).toEqual([]);
});
