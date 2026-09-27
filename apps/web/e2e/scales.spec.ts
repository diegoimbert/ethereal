// Scales on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → clip → piano-roll Scale… popover: project scale C Minor → "Scale notes only"
// folds the rows → a note drawn on a folded row gets that row's pitch → Custom track scale
// (the project's stays) → undo.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, newProject, pickOption } from "./ui";

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

const C_MINOR = new Set([0, 2, 3, 5, 7, 8, 10]);

test("scales: project scale, folded rows, custom track scale", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Scales ${Date.now()}`);

  // --- MIDI clip in the piano roll ----------------------------------------------------------
  const midi = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const roll = page.getByTestId("piano-roll");
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  const keys = roll.locator(".eth-pr-key");
  await expect(keys).toHaveCount(128);

  // --- Project scale C Minor from the Scale… popover, then fold ------------------------------
  const trigger = page.getByTestId("scale-trigger");
  await trigger.click();
  const popover = page.getByTestId("scale-popover");
  await pickOption(popover, "Active scale type", "Minor");
  await expect.poll(async () => (await doc(page)).settings.scale).toEqual({ root: 0, kind: "Minor" });
  await expect(trigger).toHaveText("C Minor…");
  await expect(roll.locator('.eth-pr-key[data-pitch="60"]')).toHaveAttribute("data-scale-tone", "root");

  await popover.getByRole("switch", { name: "Scale notes only" }).click();
  await expect(keys).toHaveCount(75); // 7 of 12 pitch classes over MIDI 0..127
  await page.keyboard.press("Escape");
  await expect(popover).toBeHidden();

  // --- A note drawn on a folded row gets that row's pitch ------------------------------------
  const key = roll.locator('.eth-pr-key[data-pitch="63"]');
  await key.scrollIntoViewIfNeeded();
  const keyBox = (await key.boundingBox())!;
  const gridBox = (await grid.boundingBox())!;
  await grid.dblclick({ position: { x: 12, y: keyBox.y - gridBox.y + keyBox.height / 2 } });
  await expect.poll(async () => Object.values((await doc(page)).notes).map((n) => n.pitch)).toEqual([63]);
  const note = roll.getByTestId("piano-roll-note");
  await expect(note).toHaveAttribute("data-pitch", "63");
  await expect(note).toHaveAttribute("data-scale-tone", "in");

  // --- Custom track scale: D Major on this track; the project keeps C Minor ------------------
  await trigger.click();
  await pickOption(popover, "Track scale mode", "Custom Track Scale");
  await pickOption(popover, "Active scale root", "D");
  await pickOption(popover, "Active scale type", "Major");
  await expect
    .poll(async () => (await doc(page)).tracks[midi.id]!.scale)
    .toEqual({ type: "Custom", scale: { root: 2, kind: "Major" } });
  expect((await doc(page)).settings.scale).toEqual({ root: 0, kind: "Minor" });
  // D major has no D# (63): the folded roll hides the note but keeps it in the clip.
  await expect(note).toHaveCount(0);
  expect(Object.values((await doc(page)).notes)).toHaveLength(1);
  await page.keyboard.press("Escape");

  // --- Undo the track scale edits: back to following the project -----------------------------
  await roll.focus();
  for (let i = 0; i < 3; i++) await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await doc(page)).tracks[midi.id]!.scale).toEqual({ type: "FollowProject" });
  await expect(trigger).toHaveText("C Minor…");
  await expect(note).toHaveAttribute("data-pitch", "63");
  // Every visible key is in C minor.
  const pitches = await keys.evaluateAll((els) => els.map((el) => Number((el as HTMLElement).dataset.pitch)));
  expect(pitches.every((p) => C_MINOR.has(p % 12))).toBe(true);

  expect(errors).toEqual([]);
});
