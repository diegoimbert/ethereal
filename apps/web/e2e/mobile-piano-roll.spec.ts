// The piano roll on touch screens (#522): a double-tap on a clip opens its editor in a panel over
// the arrangement (the transport stays); select mode: a double-tap on empty space adds a note, a
// drag on a selected note moves it, a drag on empty space pans; draw mode: a tap adds a note;
// pinch zooms; the close button and the system back close the panel.
import { expect, test, type Page } from "@playwright/test";
import { arrangerApi } from "./clips";
import { arrangementViewState, IPHONE, launchMobile, settled, shot, touchscreen, type Pt } from "./mobile";
import { createTrack, pianoRollNotes } from "./ui";

test.use(IPHONE);

const editor = (page: Page) => page.getByTestId("mobile-editor");
const grid = (page: Page) => page.getByTestId("piano-roll-grid");
const clipIds = (page: Page) =>
  page.evaluate(() =>
    Object.keys((window as unknown as { __ether: { state(): { project: { clips: Record<string, unknown> } } } }).__ether.state().project.clips),
  );

/** A MIDI track with a one-bar clip (a double-tap on its lane), opened by a double-tap. */
async function openEditor(page: Page) {
  const track = await createTrack(page, "Midi");
  const touch = await touchscreen(page);
  const row = (await page.locator(`.eth-arr__scroll .eth-arr-header[data-track="${track.id}"]`).boundingBox())!;
  const lanes = (await page.getByTestId("arrangement-content").boundingBox())!;
  const ppb = (await arrangementViewState(page)).pxPerBeat;
  const at = { x: lanes.x + row.width + 2 * ppb, y: row.y + row.height * 0.7 };
  await touch.doubleTap(at);
  await expect.poll(async () => (await clipIds(page)).length).toBe(1);
  const id = (await clipIds(page))[0]!;
  const r = (await arrangerApi(page, "clipRect", id))!;
  const on = { x: r.x + r.width / 2, y: r.y + r.height * 0.7 };
  await touch.sleep(400);
  await touch.doubleTap(on);
  await expect(editor(page)).toBeVisible();
  await expect(grid(page)).toBeVisible();
  await settled(page);
  return { touch, clip: id };
}

/** A point in the grid, `dx` px right of its left edge and `dy` px down from its visible top. */
async function inGrid(page: Page, dx: number, dy: number): Promise<Pt> {
  const body = (await page.locator(".eth-meditor .eth-pr__body").boundingBox())!;
  const g = (await grid(page).boundingBox())!;
  return { x: g.x + dx, y: body.y + dy };
}

test("a double-tap on a clip opens the piano roll in a panel; the transport stays; close and back close it", async ({ page }) => {
  await launchMobile(page);
  await openEditor(page);
  await expect(page.getByTestId("mobile-transport")).toBeVisible();
  const box = (await editor(page).boundingBox())!;
  const dock = (await page.getByTestId("mobile-transport").boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(dock.y + 1);
  await shot(page, "piano-roll");
  await editor(page).getByRole("button", { name: "close editor" }).tap();
  await expect(editor(page)).toHaveCount(0);
  // The action bar's "edit" opens it again; the system back closes it.
  await page.getByTestId("clip-actions").getByRole("button", { name: "edit", exact: true }).tap();
  await expect(editor(page)).toBeVisible();
  await page.goBack();
  await expect(editor(page)).toHaveCount(0);
  await expect(page.getByTestId("mobile-shell")).toBeVisible();
});

test("select mode: double-tap adds a note, a drag on the selected note moves it, a drag on empty space pans", async ({ page }) => {
  await launchMobile(page);
  const { touch } = await openEditor(page);
  const p = await inGrid(page, 50, 120);
  await touch.doubleTap(p);
  await expect.poll(async () => (await pianoRollNotes(grid(page))).length).toBe(1);
  const [n0] = await pianoRollNotes(grid(page));
  // The new note is selected: a drag on it moves it (right and up).
  expect(n0!.selected).toBe(true);
  const from = { x: n0!.x + n0!.width / 2, y: n0!.y + n0!.height / 2 };
  await touch.drag(from, { x: from.x + 80, y: from.y - 3 * n0!.height });
  await expect.poll(async () => (await pianoRollNotes(grid(page)))[0]!.pitch).toBe(n0!.pitch + 3);
  expect((await pianoRollNotes(grid(page)))[0]!.x).toBeGreaterThan(n0!.x + 40);
  // A drag down on empty space pans the keys (vertical scroll), no note is added.
  const body = page.locator(".eth-meditor .eth-pr__body");
  const top0 = await body.evaluate((el) => el.scrollTop);
  const e = await inGrid(page, 120, 120);
  await touch.drag(e, { x: e.x, y: e.y + 150 });
  expect(await body.evaluate((el) => el.scrollTop)).toBeLessThan(top0 - 100);
  expect(await pianoRollNotes(grid(page))).toHaveLength(1);
});

test("draw mode: a tap adds a note; pinch zooms time", async ({ page }) => {
  await launchMobile(page);
  const { touch } = await openEditor(page);
  await editor(page).getByRole("button", { name: /^draw/i }).tap();
  await expect(grid(page)).toHaveClass(/eth-pr-grid--draw/);
  await touch.tap(await inGrid(page, 40, 100));
  await expect.poll(async () => (await pianoRollNotes(grid(page))).length).toBe(1);
  const w0 = (await pianoRollNotes(grid(page)))[0]!.width;
  const c = await inGrid(page, 150, 200);
  await touch.pinch({ x: c.x - 30, y: c.y }, { x: c.x + 30, y: c.y }, { x: c.x - 60, y: c.y }, { x: c.x + 60, y: c.y });
  await expect.poll(async () => (await pianoRollNotes(grid(page)))[0]!.width).toBeGreaterThan(w0 * 1.6);
  expect(await pianoRollNotes(grid(page))).toHaveLength(1);
});

test("on a phone the velocity lanes are hidden until the editor's velocity toggle shows them", async ({ page }) => {
  await launchMobile(page);
  await openEditor(page);
  await expect(page.getByTestId("expression-lanes")).toBeHidden();
  const body = page.locator(".eth-meditor .eth-pr__body");
  const before = (await body.boundingBox())!.height;
  await editor(page).getByRole("button", { name: "velocity" }).tap();
  await expect(page.getByTestId("expression-lanes")).toBeVisible();
  await expect.poll(async () => (await body.boundingBox())!.height).toBeLessThan(before);
});
