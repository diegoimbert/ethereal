// The piano roll of a MIDI clip extended to the left past its content start (#878: the
// extended part was invisible there, the roll started at beat 1). Web build, real engine
// (WasmTransport → controller Worker → AudioWorklet), through the UI:
// a one-bar clip with a note at content beat 0 moved to bar 3, its left edge dragged 2 bars
// left (offset -8). The piano roll scrolls to the clip start (content -8, bar -1); a note
// drawn there lands at content -6 and plays at song beat 2; the existing note stays at
// content 0 (song beat 8) and both are drawn where they play.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { clipPart, openClip } from "./clips";
import { createTrack, launch, pianoRollNote, pianoRollNotes, pickOption } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const r3 = (x: number) => +x.toFixed(3);
const bounds = async (page: Page, id: string) => {
  const c = (await doc(page)).clips[id]!;
  return [r3(c.start), r3(c.length), r3(c.offset)];
};
const notesOf = async (page: Page, id: string) =>
  Object.values((await doc(page)).notes)
    .filter((n) => n.clip === id)
    .map((n) => ({ id: n.id, pitch: n.pitch, start: r3(n.start) }))
    .sort((a, b) => a.start - b.start);

/** Drag from (x, y) by dx px with intermediate moves (the arrangement's pointer drag). */
async function dragBy(page: Page, x: number, y: number, dx: number) {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 3 });
  await page.mouse.move(x + dx, y, { steps: 3 });
  await page.mouse.up();
}

/** Waits until `read` returns the same value twice in a row (a zoom / scroll settled). */
async function settled(read: () => Promise<number>) {
  let last = NaN;
  await expect
    .poll(async () => {
      const now = await read();
      const same = now === last;
      last = now;
      return same;
    })
    .toBe(true);
}

/**
 * Piano roll of a clip spanning `length` beats from content `offset`: zoom out until it fits,
 * scroll to its start, then the client x of a content beat (from the drawn clip boundaries).
 */
async function rollX(page: Page, offset: number, length: number): Promise<(beat: number) => number> {
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const left = (sel: string) => page.locator(sel).evaluate((el) => parseFloat((el as HTMLElement).style.left));
  const pxPerBeat = async () => ((await left('[data-testid="piano-roll-clip-end"]')) - (await left(".eth-pr__start"))) / length;
  await page.mouse.move(body.x + 70, body.y + body.height / 2);
  for (let i = 0; i < 60 && (await pxPerBeat()) * length > body.width - 120; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, 60);
    await page.keyboard.up("Control");
  }
  await settled(pxPerBeat);
  // Scroll left as far as it goes: the clip's start (its left-extended part) and no further.
  for (let i = 0; i < 10; i++) await page.mouse.wheel(-400, 0);
  await settled(() => left(".eth-pr__start"));
  const grid = (await page.getByTestId("piano-roll-grid").boundingBox())!;
  const x0 = await left(".eth-pr__start");
  const ppb = await pxPerBeat();
  return (beat) => grid.x + x0 + (beat - offset) * ppb;
}

test("piano roll: a clip extended to the left shows and edits its extended part", async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Roll extend left ${Date.now()}`);
  await pickOption(page.locator('[data-feature="arrangement"]'), "grid", "1/4");

  // --- A one-bar MIDI clip with a note at content beat 0 -----------------------------------
  const t = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).length).toBe(1);
  const id = Object.values((await doc(page)).clips)[0]!.id;
  await openClip(page, id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await pickOption(page.getByTestId("piano-roll"), "grid", "1/4");
  let x = await rollX(page, 0, 4);
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const rowY = body.y + body.height / 2;
  await page.mouse.dblclick(x(0.1), rowY);
  await expect.poll(async () => (await notesOf(page, id)).length).toBe(1);
  const [original] = await notesOf(page, id);
  expect(original!.start).toBe(0);
  const arrangement = page.locator('[data-feature="arrangement"]');
  await arrangement.focus();
  await page.keyboard.press("Escape");

  // --- Move it to bar 3, then drag its left edge 2 bars left (offset -8) --------------------
  const px = (await clipPart(page, id)).width / 4;
  const title = await clipPart(page, id, "header");
  await dragBy(page, title.x + title.width / 2, title.y + title.height / 2, 8 * px);
  await expect.poll(() => bounds(page, id)).toEqual([8, 4, 0]);
  const edge = await clipPart(page, id, "resize-start");
  await dragBy(page, edge.x + edge.width / 2, edge.y + edge.height - 4, -8 * px);
  await expect.poll(() => bounds(page, id)).toEqual([0, 12, -8]);

  // --- The piano roll shows the extended part: content -8 .. 4 ------------------------------
  // (Still open on the clip: a double-click on it would close the editor.)
  if (!(await page.getByTestId("piano-roll-grid").isVisible())) await openClip(page, id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  x = await rollX(page, -8, 12);
  const grid = page.getByTestId("piano-roll-grid");
  const gridBox = (await grid.boundingBox())!;
  // The clip start is on screen, right of the keyboard (not scrolled off to the left).
  expect(x(-8)).toBeGreaterThanOrEqual(gridBox.x - 1);
  // The existing note is drawn at content 0, 8 beats right of the clip start.
  const drawn = await pianoRollNote(grid, original!.id);
  expect(drawn.x).toBeCloseTo(x(0), 0);

  // --- Draw a note in the extended part (content -6 = song beat 2) --------------------------
  await page.mouse.dblclick(x(-6 + 0.1), rowY - 40);
  await expect.poll(async () => (await notesOf(page, id)).length).toBe(2);
  const added = (await notesOf(page, id)).find((n) => n.id !== original!.id)!;
  expect(added.start).toBe(-6);
  // Drawn where it plays.
  await expect.poll(async () => (await pianoRollNotes(grid)).some((n) => n.id === added.id)).toBe(true);
  expect((await pianoRollNote(grid, added.id)).x).toBeCloseTo(x(-6), 0);
  // It plays at song beat 2 (clip start 0 + (-6 - -8)).
  const c = (await doc(page)).clips[id]!;
  expect(r3(c.start + added.start - c.offset)).toBe(2);

  // --- Move the original note into the extended part (2 beats before content 0) -------------
  const o = await pianoRollNote(grid, original!.id);
  await dragBy(page, o.x + o.width / 2, o.y + o.height / 2, x(-2) - x(0));
  await expect.poll(async () => (await notesOf(page, id)).find((n) => n.id === original!.id)!.start).toBe(-2);
  expect((await pianoRollNote(grid, original!.id)).x).toBeCloseTo(x(-2), 0);

  expect(errors).toEqual([]);
});
