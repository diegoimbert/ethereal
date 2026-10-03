// MPE on the web build, through the UI, against the real engine (WasmTransport → controller
// Worker → AudioWorklet).
//
// MIDI track → inspector "MPE" switch (one undo step) → note bend range 24 → clip + note →
// lane picker "Note Pitch" (spans ±24 semitones) → a pencil stroke draws the note's pitch
// glide, which shows inside the note in the grid → "Note Timbre" stroke → undo.
//
// `MPE_SHOTS=<dir>` also saves the PR screenshots (dark 1440×900, plus one light).
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, newProject, pickOption, selectTrack, setNumberField } from "./ui";

const shots = process.env.MPE_SHOTS;
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

/**
 * A pencil stroke across `lane` over the span of `noteEl` (its page x range), from height
 * fraction `fy0` to `fy1` of the lane.
 */
async function strokeOver(page: Page, lane: Locator, noteEl: Locator, fy0: number, fy1: number) {
  const box = (await lane.boundingBox())!;
  const n = (await noteEl.boundingBox())!;
  await page.mouse.move(n.x + 1, box.y + box.height * fy0);
  await page.mouse.down();
  await page.mouse.move(n.x + n.width - 1, box.y + box.height * fy1, { steps: 12 });
  await page.mouse.up();
}

test("mpe: track settings, per-note pitch and timbre curves", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `MPE ${Date.now()}`);

  // --- MPE settings in the inspector ---------------------------------------------------------
  const midi = await createTrack(page, "Midi");
  await selectTrack(page, midi.name);
  const inspector = page.getByTestId("inspector");
  const settings = inspector.getByTestId("mpe-settings");
  await expect(settings).toBeVisible();
  await settings.getByRole("switch", { name: "MPE" }).click();
  await expect
    .poll(async () => (await doc(page)).tracks[midi.id]!.mpe)
    .toEqual({ zone: "Lower", member_channels: 15, note_pitch_range: 48, master_pitch_range: 2 });
  await setNumberField(settings.getByRole("spinbutton", { name: "Per-note pitch bend range" }), 24);
  await expect.poll(async () => (await doc(page)).tracks[midi.id]!.mpe?.note_pitch_range).toBe(24);
  await expect(settings.getByTestId("mpe-summary")).toHaveText("Master 1 · notes on 2–16");

  // --- A note with a pitch glide --------------------------------------------------------------
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const roll = page.getByTestId("piano-roll");
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  // A note in the middle of the visible rows.
  const gbox = (await grid.boundingBox())!;
  const scroller = page.locator(".eth-pr__body");
  const sbox = (await scroller.boundingBox())!;
  const y = sbox.y + sbox.height / 2 - gbox.y;
  // Double-click and keep holding: dragging sets the note's length (about 4 beats here).
  await page.mouse.move(gbox.x + 20, gbox.y + y);
  await page.mouse.down();
  await page.mouse.up();
  await page.mouse.down({ clickCount: 2 });
  await page.mouse.move(gbox.x + 220, gbox.y + y, { steps: 10 });
  await page.mouse.up({ clickCount: 2 });
  await expect.poll(async () => Object.keys((await doc(page)).notes).length).toBe(1);
  const note = Object.values((await doc(page)).notes)[0]!;

  await pickOption(roll, "Lane", { value: "note:Pitch" });
  const lane = roll.getByTestId("note-expression-lane");
  await expect(lane).toHaveAttribute("data-kind", "Note Pitch");
  const noteEl = grid.locator(`[data-testid="piano-roll-note"][data-note-id="${note.id}"]`);
  await strokeOver(page, lane, noteEl, 0.5, 0.3);
  await expect
    .poll(async () => Object.values((await doc(page)).note_expressions).map((e) => [e.note, e.kind]))
    .toEqual([[note.id, "Pitch"]]);
  const pitch = Object.values((await doc(page)).note_expressions)[0]!;
  // Starts at the centre (0 st) and rises (~+10 st: 0.2 of the lane height over the ±24 window).
  expect(Math.abs(pitch.points[0]!.value)).toBeLessThan(3);
  expect(Math.max(...pitch.points.map((p) => p.value))).toBeGreaterThan(5);
  expect(Math.max(...pitch.points.map((p) => p.value))).toBeLessThanOrEqual(24);
  // Drawn inside the note in the grid.
  await expect(grid.getByTestId("mpe-pitch-curve")).toHaveAttribute("data-note-id", note.id);

  // --- Timbre ---------------------------------------------------------------------------------
  await pickOption(roll, "Lane", { value: "note:Timbre" });
  await expect(lane).toHaveAttribute("data-kind", "Note Timbre");
  await strokeOver(page, lane, noteEl, 0.9, 0.2);
  await expect
    .poll(async () => Object.values((await doc(page)).note_expressions).map((e) => e.kind).sort())
    .toEqual(["Pitch", "Timbre"]);

  if (shots) {
    await pickOption(roll, "Lane", { value: "note:Pitch" });
    await page.screenshot({ path: `${shots}/mpe-piano-roll-dark.png` });
    await selectTrack(page, midi.name);
    await expect(settings).toBeVisible();
    await page.screenshot({ path: `${shots}/mpe-inspector-dark.png` });
  }

  // One undo removes the timbre stroke; the pitch glide stays.
  await page.keyboard.press("ControlOrMeta+z");
  await expect
    .poll(async () => Object.values((await doc(page)).note_expressions).map((e) => e.kind))
    .toEqual(["Pitch"]);

  // MPE off: back to a plain track (the curves stay, they play on any MIDI track).
  await selectTrack(page, midi.name);
  await settings.getByRole("switch", { name: "MPE" }).click();
  await expect.poll(async () => (await doc(page)).tracks[midi.id]!.mpe ?? null).toBeNull();
  expect(Object.values((await doc(page)).note_expressions)).toHaveLength(1);

  if (shots) {
    await settings.getByRole("switch", { name: "MPE" }).click();
    await expect.poll(async () => (await doc(page)).tracks[midi.id]!.mpe?.note_pitch_range).toBe(48);
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
    await page.screenshot({ path: `${shots}/mpe-inspector-light.png` });
    await openClip(page, clip.id);
    await pickOption(roll, "Lane", { value: "note:Pitch" });
    await page.screenshot({ path: `${shots}/mpe-piano-roll-light.png` });
  }

  expect(errors).toEqual([]);
});
