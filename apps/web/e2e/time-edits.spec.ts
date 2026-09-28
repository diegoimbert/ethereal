// Time-selection edits on the web build, through the UI, against the real engine
// (WasmTransport → controller Worker → AudioWorklet): a drag over two tracks' lanes makes a
// time selection; its context menu deletes the time (later clips move left), undo restores
// it, ⌘E splits both tracks at the selection edges, ⇧⌘I inserts silence, ⇧⌘D duplicates the
// selection, and a
// frozen track refuses a time edit with the engine's message.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots.
import { expect, test, type Page } from "@playwright/test";
import type { Project, TrackId } from "@/generated";
import { createTrack, newProject } from "./ui";

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

/** A track's clips as sorted [start, length] (rounded). */
const spans = async (page: Page, track: TrackId): Promise<number[][]> =>
  Object.values((await doc(page)).clips)
    .filter((c) => c.track === track)
    .map((c) => [+c.start.toFixed(3), +c.length.toFixed(3)])
    .sort((a, b) => a[0]! - b[0]!);

const shot = async (page: Page, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await page.locator('[data-feature="arrangement"]').screenshot({ path: `${dir}/${name}.png` });
};

test("time edits: select time over tracks, delete time, undo, split at the edges, duplicate", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Time edits ${Date.now()}`);

  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  const laneA = page.locator(`[data-lane="${a.id}"]`);
  const laneB = page.locator(`[data-lane="${b.id}"]`);

  // One bar clips by double-clicking the lanes: A gets bars 1 and 2, B bar 1.
  await laneA.dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, a.id)).toEqual([[0, 4]]);
  const clipA = Object.values((await doc(page)).clips).find((c) => c.track === a.id)!;
  const px = (await page.locator(`[data-clip-id="${clipA.id}"]`).boundingBox())!.width / 4;
  await laneA.dblclick({ position: { x: 5 * px, y: 30 } });
  await laneB.dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 4],
    [4, 4],
  ]);
  await expect.poll(() => spans(page, b.id)).toEqual([[0, 4]]);
  // Escape closes the piano roll the double-click opened (screenshots show the lanes).
  await page.locator('[data-feature="arrangement"]').focus();
  await page.keyboard.press("Escape");

  // Drag over both lanes from beat 2 to beat 6 (clip bodies let presses through).
  const selectTime = async (from: number, to: number) => {
    const boxA = (await laneA.boundingBox())!;
    const boxB = (await laneB.boundingBox())!;
    await page.mouse.move(boxA.x + from * px, boxA.y + boxA.height - 8);
    await page.mouse.down();
    await page.mouse.move(boxA.x + ((from + to) / 2) * px, boxA.y + boxA.height, { steps: 3 });
    await page.mouse.move(boxB.x + to * px, boxB.y + boxB.height - 8, { steps: 3 });
    await page.mouse.up();
    await expect(page.getByTestId("time-selection")).toHaveCount(2);
  };
  await selectTime(2, 6);
  await shot(page, "time-selection");

  // Right-click inside the selection: the time menu. Delete Time.
  const boxA = (await laneA.boundingBox())!;
  await page.mouse.click(boxA.x + 3 * px, boxA.y + boxA.height - 8, { button: "right" });
  const menu = page.getByRole("menu");
  await expect(menu.getByText("Delete Time")).toBeVisible();
  await shot(page, "time-selection-menu");
  await menu.getByText("Delete Time").click();
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 2],
    [2, 2],
  ]);
  await expect.poll(() => spans(page, b.id)).toEqual([[0, 2]]);
  await expect(page.getByTestId("time-selection")).toHaveCount(0);

  // One undo step.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 4],
    [4, 4],
  ]);
  await expect.poll(() => spans(page, b.id)).toEqual([[0, 4]]);

  // ⌘E splits both tracks at the selection edges (one undo step).
  await selectTime(2, 6);
  await page.keyboard.press("ControlOrMeta+e");
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 2],
    [2, 2],
    [4, 2],
    [6, 2],
  ]);
  await expect.poll(() => spans(page, b.id)).toEqual([
    [0, 2],
    [2, 2],
  ]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 4],
    [4, 4],
  ]);

  // ⇧⌘I inserts the selection's length of silence at its start (plain ⌘I is Import audio…).
  await selectTime(2, 6);
  await page.keyboard.press("ControlOrMeta+Shift+i");
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 2],
    [6, 2],
    [8, 4],
  ]);
  await expect.poll(() => spans(page, b.id)).toEqual([
    [0, 2],
    [6, 2],
  ]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => spans(page, a.id)).toEqual([
    [0, 4],
    [4, 4],
  ]);

  // ⇧⌘D duplicates the selection right after it; the rest moves right.
  await selectTime(2, 6);
  await page.keyboard.press("ControlOrMeta+Shift+d");
  await expect.poll(() => spans(page, b.id)).toEqual([
    [0, 4],
    [6, 2],
  ]);
  await expect.poll(async () => (await spans(page, a.id)).length).toBe(5);
  await shot(page, "time-duplicate");

  // A frozen track refuses time edits (freeze-bounce's check); the engine's message shows.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => spans(page, b.id)).toEqual([[0, 4]]);
  await page.getByRole("group", { name: `${b.name} track` }).click({ button: "right", position: { x: 6, y: 10 } });
  await page.getByRole("menuitem", { name: "Freeze Track", exact: true }).click();
  await expect.poll(async () => (await doc(page)).tracks[b.id]!.freeze !== undefined, { timeout: 60_000 }).toBe(true);
  await selectTime(2, 6);
  await page.keyboard.press("ControlOrMeta+Shift+Backspace");
  await expect(page.getByTestId("time-edit-notice")).toContainText("frozen");
  await shot(page, "time-frozen-notice");
  expect(await spans(page, b.id)).toEqual([[0, 4]]);
  expect(await spans(page, a.id)).toEqual([
    [0, 4],
    [4, 4],
  ]);

  expect(errors).toEqual([]);
});
