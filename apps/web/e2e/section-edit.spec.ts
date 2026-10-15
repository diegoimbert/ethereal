// Section copy/paste/duplicate (owner report, `section-edit`) on the web build, through the
// UI, against the real engine (WasmTransport → controller Worker → AudioWorklet). A MIDI
// clip with notes at beats 0 and 3. In the arrangement a plain drag selects the clip (a
// Shift-drag would make a time section, #268); ⌘D tiles the clip. In the piano roll a
// Shift-drag makes the section; ⌘C / ⌘V and ⌘D tile it with its exact
// length (gaps kept) and the clip grows. The piano-roll section is free like the arrangement's
// (owner request): never widened to the notes it selects.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots (dark).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project, TrackId } from "@/generated";
import { clipPart, isClipSelected, openClip } from "./clips";
import { createTrack, launch, newProject, pickOption } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const r3 = (x: number) => +x.toFixed(3);

/** A track's clips as sorted [start, length] (rounded). */
const spans = async (page: Page, track: TrackId): Promise<number[][]> =>
  Object.values((await doc(page)).clips)
    .filter((c) => c.track === track)
    .map((c) => [r3(c.start), r3(c.length)])
    .sort((a, b) => a[0]! - b[0]!);

/** Song positions of the notes that play on `track` (start inside their clip's window). */
const audible = async (page: Page, track: TrackId): Promise<number[]> => {
  const p = await doc(page);
  const out: number[] = [];
  for (const c of Object.values(p.clips).filter((c) => c.track === track)) {
    for (const n of Object.values(p.notes).filter((n) => n.clip === c.id)) {
      const rel = n.start - c.offset;
      if (rel >= -1e-6 && rel < c.length - 1e-6) out.push(r3(c.start + rel));
    }
  }
  return out.sort((a, b) => a - b);
};

/** A clip's note starts (content beats), sorted. */
const noteStarts = async (page: Page, clip: string): Promise<number[]> =>
  Object.values((await doc(page)).notes)
    .filter((n) => n.clip === clip)
    .map((n) => r3(n.start))
    .sort((a, b) => a - b);

const shot = async (target: Locator, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await target.screenshot({ path: `${dir}/${name}.png` });
};

/** Piano-roll geometry: the client x of a content beat, and a y on a visible row. */
async function rollGeometry(page: Page, clipLength: number) {
  const grid = page.getByTestId("piano-roll-grid");
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const gridBox = (await grid.boundingBox())!;
  const left = (sel: string) => page.locator(sel).evaluate((el) => parseFloat((el as HTMLElement).style.left));
  const x0 = await left(".eth-pr__start");
  const pxPerBeat = ((await left('[data-testid="piano-roll-clip-end"]')) - x0) / clipLength;
  return {
    x: (beat: number) => gridBox.x + x0 + beat * pxPerBeat,
    y: body.y + body.height / 2,
    rowY: (dy: number) => body.y + body.height / 2 + dy,
  };
}

/**
 * Zooms the piano roll out (ctrl + wheel near its left edge) until `beats` fit in the left
 * half of the visible grid, so every gesture lands on screen. The clip is 4 beats long.
 */
async function fitRoll(page: Page, beats: number) {
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const left = (sel: string) => page.locator(sel).evaluate((el) => parseFloat((el as HTMLElement).style.left));
  const pxPerBeat = async () => ((await left('[data-testid="piano-roll-clip-end"]')) - (await left(".eth-pr__start"))) / 4;
  const room = (body.width - 64) / 2;
  await page.mouse.move(body.x + 70, body.y + body.height / 2);
  for (let i = 0; i < 60 && (await pxPerBeat()) * beats > room; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, 60);
    await page.keyboard.up("Control");
  }
  expect((await pxPerBeat()) * beats).toBeLessThanOrEqual(room);
  // Wait for the zoom to settle (the view applies wheel steps over frames).
  let last = NaN;
  await expect
    .poll(async () => {
      const now = await pxPerBeat();
      const settled = now === last;
      last = now;
      return settled;
    })
    .toBe(true);
}

/** Two notes (beats 0 and 3) drawn by double-clicking the open piano roll's grid. */
async function drawNotes(page: Page, clip: string) {
  // A fixed 1/4 grid: drawn notes and sections snap to beats whatever the zoom.
  await pickOption(page.getByTestId("piano-roll"), "grid", "1/4");
  await fitRoll(page, 16);
  const g = await rollGeometry(page, 4);
  for (const beat of [0, 3]) {
    const before = (await noteStarts(page, clip)).length;
    await page.mouse.dblclick(g.x(beat + 0.1), g.y);
    await expect.poll(async () => (await noteStarts(page, clip)).length).toBe(before + 1);
  }
  await expect.poll(() => noteStarts(page, clip)).toEqual([0, 3]);
}

test("section edit: copy/paste a section (not the clip) and duplicate it with its exact length", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Section edit ${Date.now()}`);

  // --- A one-bar MIDI clip with notes at beats 0 and 3 --------------------------------------
  const t = await createTrack(page, "Midi");
  const lane = page.locator(`[data-lane="${t.id}"]`);
  await lane.dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, t.id)).toEqual([[0, 4]]);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!;
  await openClip(page, clip.id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await drawNotes(page, clip.id);
  expect(await audible(page, t.id)).toEqual([0, 3]);

  // Back to the arrangement (Escape closes the piano roll the double-click opened).
  const arrangement = page.locator('[data-feature="arrangement"]');
  await arrangement.focus();
  await page.keyboard.press("Escape");
  const px = (await clipPart(page, clip.id)).width / 4;

  // --- A plain drag over the lane draws no time section (a Shift-drag would, #268) ---------
  // It selects the clip it crosses.
  const box = (await lane.boundingBox())!;
  await page.mouse.move(box.x + 2 * px, box.y + box.height - 8);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 * px, box.y + box.height - 6, { steps: 3 });
  await page.mouse.move(box.x + 6 * px, box.y + box.height - 8, { steps: 3 });
  await page.mouse.up();
  await expect(page.getByTestId("time-selection")).toHaveCount(0);
  await expect.poll(() => isClipSelected(page, clip.id)).toBe(true);

  // --- Plain ⌘D three times on the selected clip: it tiles, gaps kept -----------------------
  for (let i = 1; i <= 3; i++) {
    await page.keyboard.press("ControlOrMeta+d");
    await expect.poll(async () => (await spans(page, t.id)).length).toBe(1 + i);
    if (i === 2) await shot(arrangement, "section-duplicate-arrangement");
  }
  await expect.poll(() => audible(page, t.id)).toEqual([0, 3, 4, 7, 8, 11, 12, 15]);
  expect(await spans(page, t.id)).toEqual([
    [0, 4],
    [4, 4],
    [8, 4],
    [12, 4],
  ]);
  // ⌘C copies the clip; a click at beat 20 places the marker; ⌘V pastes the clip there.
  await page.keyboard.press("ControlOrMeta+c");
  await page.mouse.click(box.x + 20 * px, box.y + box.height - 8);
  await expect(page.getByTestId("insert-marker")).toHaveAttribute("data-beats", "20");
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(() => audible(page, t.id)).toEqual([0, 3, 4, 7, 8, 11, 12, 15, 20, 23]);

  // --- Piano roll: a fresh clip on another track, notes at 0 and 3 ------------------------
  const t2 = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t2.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, t2.id)).toEqual([[0, 4]]);
  const clip2 = Object.values((await doc(page)).clips).find((c) => c.track === t2.id)!;
  await openClip(page, clip2.id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await drawNotes(page, clip2.id);

  // A Shift-drag (#268: a plain drag only selects notes) from just after beat 0 to beat 3.9:
  // the section 0..4.
  const g = await rollGeometry(page, 4);
  const roll = page.getByTestId("piano-roll");
  await page.mouse.move(g.x(0.05), g.rowY(-40));
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(g.x(2), g.rowY(0), { steps: 3 });
  await page.mouse.move(g.x(3.95), g.rowY(40), { steps: 3 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  await expect(page.getByTestId("piano-roll-section")).toBeVisible();
  // ⌘C / ⌘V pastes the section right after itself; ⌘D twice keeps tiling. The clip grows.
  await page.keyboard.press("ControlOrMeta+c");
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(() => noteStarts(page, clip2.id)).toEqual([0, 3, 4, 7]);
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(() => noteStarts(page, clip2.id)).toEqual([0, 3, 4, 7, 8, 11]);
  await shot(roll, "section-duplicate-piano-roll");
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(() => noteStarts(page, clip2.id)).toEqual([0, 3, 4, 7, 8, 11, 12, 15]);
  await expect.poll(() => spans(page, t2.id)).toEqual([[0, 16]]);
  expect(await audible(page, t2.id)).toEqual([0, 3, 4, 7, 8, 11, 12, 15]);
  // One undo step per duplicate (the notes and the clip growth together).
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => noteStarts(page, clip2.id)).toEqual([0, 3, 4, 7, 8, 11]);
  await expect.poll(() => spans(page, t2.id)).toEqual([[0, 12]]);

  expect(errors).toEqual([]);
});

test("insert marker: a lane click while playing sets the paste point, never the playhead", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Insert marker ${Date.now()}`);

  // A one-bar MIDI clip; select it with a plain drag (no time section) and copy it.
  const t = await createTrack(page, "Midi");
  const lane = page.locator(`[data-lane="${t.id}"]`);
  await lane.dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, t.id)).toEqual([[0, 4]]);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!;
  const arrangement = page.locator('[data-feature="arrangement"]');
  await arrangement.focus();
  await page.keyboard.press("Escape");
  const px = (await clipPart(page, clip.id)).width / 4;
  const box = (await lane.boundingBox())!;
  await page.mouse.move(box.x + 4 * px + 3, box.y + box.height - 8);
  await page.mouse.down();
  await page.mouse.move(box.x + 2 * px, box.y + box.height - 6, { steps: 3 });
  await page.mouse.move(box.x + 1, box.y + box.height - 8, { steps: 3 });
  await page.mouse.up();
  await expect(page.getByTestId("time-selection")).toHaveCount(0);
  await expect.poll(() => isClipSelected(page, clip.id)).toBe(true);
  await page.keyboard.press("ControlOrMeta+c");

  // Play, then click the lane at beat 12: the marker goes there, the playhead keeps playing
  // from where it was (well left of the marker).
  const transportBar = page.locator('[data-feature="transport-bar"]');
  await transportBar.getByRole("button", { name: "play", exact: true }).click();
  const playing = () =>
    page.evaluate(() => (window as unknown as { __ether: { state(): { transport: { playing: boolean } | null } } }).__ether.state().transport?.playing);
  await expect.poll(playing).toBe(true);
  await page.mouse.click(box.x + 12 * px, box.y + box.height - 8);
  const marker = page.getByTestId("insert-marker");
  await expect(marker).toHaveCount(1);
  await expect(marker).toHaveAttribute("data-beats", "12");
  const markerX = (await marker.boundingBox())!.x;
  const playheadX = async () => (await page.getByTestId("playhead-line").first().boundingBox())!.x;
  expect(await playheadX()).toBeLessThan(markerX - 2 * px);
  await shot(arrangement, "insert-marker-arrangement");

  // ⌘V pastes the section at the marker (not at the playhead), still playing.
  await arrangement.focus();
  await page.keyboard.press("ControlOrMeta+v");
  await expect
    .poll(() => spans(page, t.id))
    .toEqual([
      [0, 4],
      [12, 4],
    ]);
  expect(await playheadX()).toBeLessThan(markerX - px);

  // Stop: the playhead goes to the marker placed while playing, so Play starts from it.
  await transportBar.getByRole("button", { name: "stop" }).first().click();
  await expect.poll(playing).toBe(false);
  await expect.poll(async () => Math.abs((await playheadX()) - markerX)).toBeLessThan(4);

  // Piano roll: a click on empty grid places its marker too.
  await openClip(page, clip.id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await pickOption(page.getByTestId("piano-roll"), "grid", "1/4");
  await fitRoll(page, 8);
  const g = await rollGeometry(page, 4);
  await page.mouse.click(g.x(2.1), g.y);
  await expect(page.getByTestId("piano-roll-marker")).toHaveAttribute("data-beats", "2");
  await shot(page.getByTestId("piano-roll"), "insert-marker-piano-roll");

  expect(errors).toEqual([]);
});

test("piano-roll section: free, like the arrangement's (not widened to the notes it selects)", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Section clamp ${Date.now()}`);

  const t = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, t.id)).toEqual([[0, 4]]);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!;
  await openClip(page, clip.id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await drawNotes(page, clip.id);
  // The drawn notes' ends (their length is the grid step): the last one runs past 3.5.
  const notes = Object.values((await doc(page)).notes).filter((n) => n.clip === clip.id);
  const lastEnd = r3(Math.max(...notes.map((n) => n.start + n.duration)));
  expect(lastEnd).toBeGreaterThan(3.5);

  const g = await rollGeometry(page, 4);
  const roll = page.getByTestId("piano-roll");
  const strip = page.getByTestId("piano-roll-loopbar");
  const stripBox = (await strip.boundingBox())!;
  const stripY = stripBox.y + stripBox.height / 2;
  /** The section's [start, end] in content beats, read from the strip's drawing. */
  const section = async (): Promise<number[]> => {
    const x0 = await page.locator(".eth-pr__start").evaluate((el) => parseFloat((el as HTMLElement).style.left));
    const end = await page.getByTestId("piano-roll-clip-end").evaluate((el) => parseFloat((el as HTMLElement).style.left));
    const ppb = (end - x0) / 4;
    const s = page.getByTestId("piano-roll-strip-section");
    const left = await s.evaluate((el) => parseFloat((el as HTMLElement).style.left));
    const width = await s.evaluate((el) => parseFloat((el as HTMLElement).style.width));
    return [r3((left - x0) / ppb), r3((left + width - x0) / ppb)];
  };
  /** Drag on the strip from beat `from` to `to` with Alt held (no snapping). */
  const stripDrag = async (from: number, to: number) => {
    await page.keyboard.down("Alt");
    await page.mouse.move(g.x(from), stripY);
    await page.mouse.down();
    await page.mouse.move(g.x((from + to) / 2), stripY, { steps: 3 });
    await page.mouse.move(g.x(to), stripY, { steps: 3 });
    await page.mouse.up();
    await page.keyboard.up("Alt");
  };

  // From 0 to 3.5: both notes are in it, the one at 3 lasts past 3.5: the section stays 0..3.5.
  await stripDrag(0, 3.5);
  await expect.poll(section).toEqual([0, 3.5]);
  await shot(roll, "piano-roll-section-clamped-dark");
  await page.emulateMedia({ colorScheme: "light" });
  await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), "light");
  await shot(roll, "piano-roll-section-clamped-light");
  await page.emulateMedia({ colorScheme: "dark" });
  await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), "dark");

  // Past the last note: allowed.
  await stripDrag(0, 6);
  await expect.poll(section).toEqual([0, 6]);

  // A plain marquee over the note at 0 only selects it, and makes no section (#268: selection
  // and section are separate). A strip section from 0 to 0.6 selects that note and is 0..0.6.
  await page.mouse.move(g.x(0.3), g.rowY(-40));
  await page.mouse.down();
  await page.mouse.move(g.x(0.45), g.rowY(0), { steps: 3 });
  await page.mouse.move(g.x(0.6), g.rowY(40), { steps: 3 });
  await page.mouse.up();
  await expect(page.getByTestId("piano-roll-strip-section")).toHaveCount(0);
  await stripDrag(0, 0.6);
  await expect.poll(section).toEqual([0, 0.6]);

  expect(errors).toEqual([]);
});

test("piano-roll section: undoing a duplicate brings the section back where it was", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page);
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  const t = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, t.id)).toEqual([[0, 4]]);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!;
  await openClip(page, clip.id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await drawNotes(page, clip.id);

  const g = await rollGeometry(page, 4);
  const box = page.getByTestId("piano-roll-section");
  const where = () => box.evaluate((el) => [(el as HTMLElement).style.left, (el as HTMLElement).style.width].join(" "));
  // A Shift-drag section over both notes, 0..4.
  await page.keyboard.down("Shift");
  await page.mouse.move(g.x(0), g.rowY(-30));
  await page.mouse.down();
  await page.mouse.move(g.x(2), g.rowY(0), { steps: 3 });
  await page.mouse.move(g.x(4), g.rowY(30), { steps: 3 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  await expect(box).toBeVisible();
  const before = await where();

  await page.getByTestId("piano-roll").focus();
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(() => noteStarts(page, clip.id)).toEqual([0, 3, 4, 7]);
  await expect.poll(where).not.toBe(before);

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => noteStarts(page, clip.id)).toEqual([0, 3]);
  await expect.poll(where).toBe(before);
});
