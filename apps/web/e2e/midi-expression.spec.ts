// MIDI expression lanes on the web build, through the UI, against the real engine
// (WasmTransport → controller Worker → AudioWorklet).
//
// MIDI track → clip → piano roll → lane picker "Pitch Bend" → a pencil stroke creates the
// lane and draws the curve (one gesture) → the curve shows → one undo removes the stroke
// and the lane it created. Then a Mod Wheel stroke, and per-note pressure over a note.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, launch, newProject, pickOption } from "./ui";

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

/** A pencil stroke across `lane` from (fx0, fy0) to (fx1, fy1), fractions of its box. */
async function stroke(page: Page, lane: ReturnType<Page["getByTestId"]>, [fx0, fy0]: [number, number], [fx1, fy1]: [number, number]) {
  const box = (await lane.boundingBox())!;
  const at = (fx: number, fy: number) => [box.x + box.width * fx, box.y + box.height * fy] as const;
  await page.mouse.move(...at(fx0, fy0));
  await page.mouse.down();
  await page.mouse.move(...at(fx1, fy1), { steps: 12 });
  await page.mouse.up();
}

test("midi expression: draw a pitch bend lane, undo, mod wheel, note pressure", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Expression ${Date.now()}`);

  const midi = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const roll = page.getByTestId("piano-roll");
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await expect(roll.getByTestId("piano-roll-velocity")).toBeVisible();

  // --- Pitch bend: a stroke creates the lane and draws it (one gesture) ----------------------
  await pickOption(roll, "Lane", { value: "PitchBend" });
  const lane = roll.getByTestId("expression-lane");
  await expect(lane).toHaveAttribute("data-kind", "Pitch Bend");
  await stroke(page, lane, [0.05, 0.1], [0.3, 0.9]);
  await expect.poll(async () => Object.values((await doc(page)).expression_lanes).length).toBe(1);
  const bend = Object.values((await doc(page)).expression_lanes)[0]!;
  expect(bend.clip).toBe(clip.id);
  expect(bend.kind).toEqual({ type: "PitchBend" });
  expect(bend.points.length).toBeGreaterThan(3);
  expect(bend.points[0]!.value).toBeGreaterThan(0.5);
  expect(bend.points[bend.points.length - 1]!.value).toBeLessThan(-0.5);
  await expect(lane.getByTestId("expression-curve")).toBeVisible();

  // One undo removes the whole stroke, lane included.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => Object.values((await doc(page)).expression_lanes).length).toBe(0);
  await expect(lane.getByTestId("expression-curve")).toHaveCount(0);

  // --- Mod wheel ------------------------------------------------------------------------------
  await pickOption(roll, "Lane", { value: "cc1" });
  await expect(lane).toHaveAttribute("data-kind", "CC 1 Mod Wheel");
  await stroke(page, lane, [0.1, 0.8], [0.5, 0.2]);
  await expect
    .poll(async () => Object.values((await doc(page)).expression_lanes).map((l) => l.kind))
    .toEqual([{ type: "Cc", controller: 1 }]);

  // --- Note pressure over a note ---------------------------------------------------------------
  const grid = page.getByTestId("piano-roll-grid");
  await grid.dblclick({ position: { x: 20, y: 200 } });
  await expect.poll(async () => Object.keys((await doc(page)).notes).length).toBe(1);
  await pickOption(roll, "Lane", { value: "note:Pressure" });
  const pressure = roll.getByTestId("note-expression-lane");
  await expect(pressure).toBeVisible();
  const note = Object.values((await doc(page)).notes)[0]!;
  await stroke(page, pressure, [0.001, 0.9], [0.2, 0.1]);
  await expect
    .poll(async () => Object.values((await doc(page)).note_expressions).map((e) => [e.note, e.kind]))
    .toEqual([[note.id, "Pressure"]]);

  expect(errors).toEqual([]);
});
