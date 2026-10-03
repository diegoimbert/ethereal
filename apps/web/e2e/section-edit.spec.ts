// Section copy/paste/duplicate (owner report, `section-edit`) on the web build, through the
// UI, against the real engine (WasmTransport → controller Worker → AudioWorklet). The owner's
// steps: a MIDI clip with notes at beats 0 and 3; drag a time selection over a section;
// plain ⌘C / ⌘V copy and paste only that section (not the whole clip), and ⌘D three times
// tiles it with its exact length (gaps kept: notes at 0,3,4,7,8,11,12,15). Then the same
// in the piano roll: a marquee section, ⌘C / ⌘V and ⌘D, and the clip grows.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots (dark).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project, TrackId } from "@/generated";
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
  for (let i = 0; i < 20 && (await pxPerBeat()) * beats > room; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, 200);
    await page.keyboard.up("Control");
  }
  expect((await pxPerBeat()) * beats).toBeLessThanOrEqual(room);
}

/** Two notes (beats 0 and 3) drawn by double-clicking the open piano roll's grid. */
async function drawNotes(page: Page, clip: string) {
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

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
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
  const px = (await page.locator(`[data-clip-id="${clip.id}"]`).boundingBox())!.width / 4;
  /** Drag over the lane from beat `from` to `to`; `nudge` px keeps presses off clip edges. */
  const selectTime = async (from: number, to: number, nudge: [number, number] = [0, 0]) => {
    const box = (await lane.boundingBox())!;
    await page.mouse.move(box.x + from * px + nudge[0], box.y + box.height - 8);
    await page.mouse.down();
    await page.mouse.move(box.x + ((from + to) / 2) * px, box.y + box.height - 6, { steps: 3 });
    await page.mouse.move(box.x + to * px + nudge[1], box.y + box.height - 8, { steps: 3 });
    await page.mouse.up();
    await expect(page.getByTestId("time-selection")).toHaveCount(1);
  };

  // --- Owner step 1: select beats 2..4 (half the clip), plain ⌘C, click at beat 8, ⌘V ------
  // Only the section is copied: a 2-beat clip lands at 8 with just the note at 3 (→ 9).
  await selectTime(2, 4);
  await page.keyboard.press("ControlOrMeta+c");
  const box = (await lane.boundingBox())!;
  await page.mouse.click(box.x + 8 * px, box.y + box.height - 8);
  await expect(page.getByTestId("time-selection")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(() => spans(page, t.id)).toEqual([
    [0, 4],
    [8, 2],
  ]);
  expect(await audible(page, t.id)).toEqual([0, 3, 9]);
  const pasted = Object.values((await doc(page)).clips).find((c) => c.track === t.id && c.start === 8)!;
  expect(await noteStarts(page, pasted.id)).toHaveLength(1);
  // One undo step.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => spans(page, t.id)).toEqual([[0, 4]]);

  // --- Owner step 2: select the bar, plain ⌘D three times: the section tiles, gaps kept -----
  // From just past the clip's end (not its resize handle) back to the lane's start.
  await selectTime(4, 0, [3, -20]);
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
  // The selection followed the copies (12..16): ⌘V pastes the section after it (16..20).
  await page.keyboard.press("ControlOrMeta+c");
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(() => audible(page, t.id)).toEqual([0, 3, 4, 7, 8, 11, 12, 15, 16, 19]);

  // --- Piano roll: a fresh clip on another track, notes at 0 and 3 ------------------------
  const t2 = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t2.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, t2.id)).toEqual([[0, 4]]);
  const clip2 = Object.values((await doc(page)).clips).find((c) => c.track === t2.id)!;
  await openClip(page, clip2.id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await drawNotes(page, clip2.id);

  // A marquee from just after beat 0 to beat 3.9 over both notes: the section 0..4.
  const g = await rollGeometry(page, 4);
  const roll = page.getByTestId("piano-roll");
  await page.mouse.move(g.x(0.05), g.rowY(-40));
  await page.mouse.down();
  await page.mouse.move(g.x(2), g.rowY(0), { steps: 3 });
  await page.mouse.move(g.x(3.95), g.rowY(40), { steps: 3 });
  await page.mouse.up();
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
