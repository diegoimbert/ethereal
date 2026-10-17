// A note drawn in the piano roll takes the velocity of the last touched note (owner request,
// Ableton-like), on the web build, through the UI, against the real engine. A MIDI clip with
// two notes drawn at the default velocity; select one, Alt-drag its velocity down; a note
// double-clicked next has that velocity, and one ⌘Z removes it.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip } from "./clips";
import { clickPianoRollNote, createTrack, launch, pianoRollNote } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

/** A clip's notes, sorted by start. */
const notesOf = async (page: Page, clip: string): Promise<Note[]> =>
  Object.values((await doc(page)).notes)
    .filter((n) => n.clip === clip)
    .sort((a, b) => a.start - b.start);

test("a double-clicked note takes the velocity of the last selected note", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, `Last velocity ${Date.now()}`);

  // --- A MIDI clip with two notes at the default velocity ----------------------------------------
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
  expect(drawn.map((n) => Math.round(n.velocity * 127))).toEqual([100, 100]);

  // --- Select the first note, Alt-drag its velocity down 54 px (-0.3 of the 180 px range) --------
  await clickPianoRollNote(page, await pianoRollNote(grid, drawn[0]!.id));
  const nb = await pianoRollNote(grid, drawn[0]!.id);
  const cx = nb.x + nb.width / 2;
  const cy = nb.y + nb.height / 2;
  await page.mouse.move(cx, cy);
  await page.keyboard.down("Alt");
  await expect(grid).toHaveClass(/eth-pr-grid--alt/);
  await page.mouse.down();
  await page.mouse.move(cx, cy + 54, { steps: 6 });
  await page.mouse.up();
  await page.keyboard.up("Alt");
  const want = drawn[0]!.velocity - 0.3;
  await expect.poll(async () => Math.abs((await notesOf(page, clip))[0]!.velocity - want) < 0.005).toBe(true);
  // Click the other note and back: the last-clicked one (the first) is the one that counts.
  await clickPianoRollNote(page, await pianoRollNote(grid, drawn[1]!.id));
  await clickPianoRollNote(page, await pianoRollNote(grid, drawn[0]!.id));

  // --- A double-click on empty space: the new note has the edited note's velocity ----------------
  await page.mouse.dblclick(gbox.x + 400, y - 60);
  await expect.poll(async () => (await notesOf(page, clip)).length).toBe(3);
  const added = (await notesOf(page, clip)).find((n) => !drawn.some((d) => d.id === n.id))!;
  expect(Math.abs(added.velocity - want)).toBeLessThan(0.005);

  // --- One undo step removes it ---------------------------------------------------------------
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await notesOf(page, clip)).length).toBe(2);

  expect(errors).toEqual([]);
});
