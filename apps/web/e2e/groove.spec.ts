// Groove on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → clip → draw notes in the piano roll → Quantize… popover (1/8, 100%, swing
// 100%): starts land on eighths, odd ones delayed by 1/6 beat → right-click a note →
// Humanize (the clicked note moves) → Groove tab: project swing 50% on 1/8.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip } from "./clips";

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

const notes = async (page: Page): Promise<Note[]> =>
  Object.values((await doc(page)).notes).sort((a, b) => a.pitch - b.pitch || a.start - b.start);

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

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByLabel("New project name").fill(`Groove ${Date.now()}`);
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "New" }).click();
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);

  // --- MIDI clip in the piano roll, with notes off the grid ---------------------------------
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create MIDI track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Midi")).toBe(true);
  const midi = Object.values((await doc(page)).tracks).find((t) => t.kind === "Midi")!;
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  // Grid off, so drawn notes start wherever we click.
  await page.getByTestId("piano-roll").getByRole("combobox", { name: "Grid", exact: true }).click();
  await page.getByRole("option", { name: "Off", exact: true }).click();
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
  await page.getByRole("button", { name: "Quantize…" }).click();
  await page.getByRole("combobox", { name: "Quantize grid" }).click();
  await page.getByRole("option", { name: "1/8", exact: true }).click();
  const swing = page.getByRole("spinbutton", { name: "Swing" });
  await swing.fill("100");
  await swing.press("Enter");
  await page.getByRole("button", { name: "Quantize 3 selected" }).click();
  await expect.poll(async () => (await notes(page)).every((n) => onSwungEighths(n.start))).toBe(true);

  // --- Right-click a note → Humanize: only that note changes ---------------------------------
  const before = await notes(page);
  await page.getByTestId("piano-roll").press("Escape");
  await page.getByTestId("piano-roll-note").first().click({ button: "right" });
  await page.getByRole("menuitem", { name: /Humanize/ }).click();
  await expect
    .poll(async () => {
      const after = await notes(page);
      return after.filter((n, i) => n.start !== before[i]!.start || n.velocity !== before[i]!.velocity).length;
    })
    .toBe(1);

  // --- Groove tab: project playback swing ----------------------------------------------------
  await page.getByRole("tablist", { name: "Editors" }).getByRole("tab", { name: "Groove" }).click();
  const amount = page.getByRole("spinbutton", { name: "Project swing" });
  await amount.fill("50");
  await amount.press("Enter");
  await page.getByRole("combobox", { name: "Swing grid" }).click();
  await page.getByRole("option", { name: "1/8", exact: true }).click();
  await expect.poll(async () => (await doc(page)).settings.swing).toBeCloseTo(0.5, 5);
  await expect.poll(async () => (await doc(page)).settings.swing_grid).toBe(0.5);

  expect(errors).toEqual([]);
});
