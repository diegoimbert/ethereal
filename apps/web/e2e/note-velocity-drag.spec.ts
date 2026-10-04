// Alt/Option-drag on a note changes its velocity (base-141, owner request, Ableton-style) on
// the web build, through the UI, against the real engine (WasmTransport → controller Worker →
// AudioWorklet). A MIDI clip with two notes, both selected; Alt-drag one of them up: both
// get louder by the same amount (a badge shows the value while dragging), nothing moves, and
// one ⌘Z undoes the whole drag. Without Alt, the same drag moves the notes.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots (dark,
// plus one light).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip } from "./clips";
import { createTrack, newProject } from "./ui";

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

/** A clip's notes, sorted by start. */
const notesOf = async (page: Page, clip: string): Promise<Note[]> =>
  Object.values((await doc(page)).notes)
    .filter((n) => n.clip === clip)
    .sort((a, b) => a.start - b.start);

const shot = async (target: Locator | Page, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await target.screenshot({ path: `${dir}/${name}.png` });
};

test("alt-drag on a selected note changes the selection's velocity as one undo step", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Velocity drag ${Date.now()}`);

  // --- A MIDI clip with two notes ---------------------------------------------------------------
  const t = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === t.id).length).toBe(1);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!.id;
  await openClip(page, clip);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  const gbox = (await grid.boundingBox())!;
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const y = body.y + body.height / 2;
  for (const x of [gbox.x + 30, gbox.x + 200]) {
    const before = (await notesOf(page, clip)).length;
    await page.mouse.dblclick(x, y);
    await expect.poll(async () => (await notesOf(page, clip)).length).toBe(before + 1);
  }
  const drawn = await notesOf(page, clip);
  const v0 = drawn.map((n) => n.velocity);
  /** The clip's velocities are within 0.005 of `want` (the engine stores f32). */
  const velocitiesNear = async (want: number[]) =>
    (await notesOf(page, clip)).every((n, i) => Math.abs(n.velocity - want[i]!) < 0.005);

  // --- Select both, then Alt-drag the first one up 36 px (+0.2 of the 180 px range) -------------
  const noteEl = (id: string) => grid.locator(`[data-testid="piano-roll-note"][data-note-id="${id}"]`);
  await noteEl(drawn[0]!.id).click();
  await noteEl(drawn[1]!.id).click({ modifiers: ["Shift"] });
  const nb = (await noteEl(drawn[0]!.id).boundingBox())!;
  const cx = nb.x + nb.width / 2;
  const cy = nb.y + nb.height / 2;
  await page.mouse.move(cx, cy);
  await page.keyboard.down("Alt");
  await expect(grid).toHaveClass(/eth-pr-grid--alt/);
  await expect.poll(() => noteEl(drawn[0]!.id).evaluate((el) => getComputedStyle(el).cursor)).toBe("ns-resize");
  // Down 72 px (-0.4), then back up to 36 px above the press (+0.2): up is louder.
  await page.mouse.down();
  await page.mouse.move(cx + 40, cy + 18, { steps: 4 });
  await page.mouse.move(cx + 40, cy + 72, { steps: 4 });
  const badge = page.getByTestId("piano-roll-velocity-badge");
  await expect(badge).toBeVisible();
  await expect.poll(() => velocitiesNear(v0.map((v) => v - 0.4))).toBe(true);
  await page.mouse.move(cx + 40, cy - 36, { steps: 6 });
  await expect.poll(() => velocitiesNear(v0.map((v) => Math.min(1, v + 0.2)))).toBe(true);
  await expect(badge).toHaveText(`Velocity ${Math.round(Math.min(1, v0[0]! + 0.2) * 127)}`);
  await shot(page, "velocity-drag-dark");
  const light = async () => {
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
    await shot(page, "velocity-drag-light");
    await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));
  };
  if (process.env.E2E_SHOTS) await light();
  await page.mouse.up();
  await page.keyboard.up("Alt");
  await expect(badge).toHaveCount(0);

  // Nothing moved; both changed by the same amount.
  const after = await notesOf(page, clip);
  expect(after.map((n) => [n.start, n.pitch, n.duration])).toEqual(drawn.map((n) => [n.start, n.pitch, n.duration]));

  // --- One undo step for the whole drag ---------------------------------------------------------
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await notesOf(page, clip)).map((n) => n.velocity)).toEqual(v0);

  // --- Without Alt, the same drag moves the notes (velocity unchanged) --------------------------
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx, cy - 40, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await notesOf(page, clip))[0]!.pitch).toBeGreaterThan(drawn[0]!.pitch);
  expect((await notesOf(page, clip)).map((n) => n.velocity)).toEqual(v0);

  expect(errors).toEqual([]);
});
