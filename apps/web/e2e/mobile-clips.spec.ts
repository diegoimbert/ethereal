// Clips on touch screens: a double-tap on an empty MIDI lane makes a clip; a tap selects
// (the action bar and the trim grips appear); a drag on a selected clip moves it at once, on an
// unselected one it pans (a long-press first moves it); wide trim handles; split / duplicate /
// copy / paste / delete from the bar; a second finger cancels a drag; edge auto-scroll.
import { expect, test, type Page } from "@playwright/test";
import { arrangerApi } from "./clips";
import { arrangementViewState, IPHONE, launchMobile, shot, touchscreen, type Pt } from "./mobile";
import { createTrack } from "./ui";

test.use(IPHONE);

interface ClipInfo {
  id: string;
  track: string;
  start: number;
  length: number;
}
const clips = (page: Page) =>
  page.evaluate(() => {
    const p = (window as unknown as { __ether: { state(): { project: { clips: Record<string, ClipInfo> } | null } } }).__ether.state().project;
    return Object.values(p?.clips ?? {}).map((c) => ({ id: c.id, track: c.track, start: c.start, length: c.length }));
  });
const clip = async (page: Page, id: string) => (await clips(page)).find((c) => c.id === id)!;
const actions = (page: Page) => page.getByTestId("clip-actions");

/** A MIDI track with a one-bar clip made by a double-tap on its empty lane, in the first bar. */
async function midiClip(page: Page): Promise<{ id: string; track: string }> {
  const track = await createTrack(page, "Midi");
  const touch = await touchscreen(page);
  const row = (await page.locator(`.eth-arr__scroll .eth-arr-header[data-track="${track.id}"]`).boundingBox())!;
  const lanes = (await page.getByTestId("arrangement-content").boundingBox())!;
  const ppb = (await arrangementViewState(page)).pxPerBeat;
  const at = { x: lanes.x + row.width + 2 * ppb + 4, y: row.y + row.height / 2 };
  await touch.doubleTap(at);
  await expect.poll(async () => (await clips(page)).length).toBe(1);
  const c = (await clips(page))[0]!;
  expect(c.start).toBe(0);
  return { id: c.id, track: track.id };
}

/** A point on a clip's body (client px), `fx` across. */
async function onClip(page: Page, id: string, fx = 0.5): Promise<Pt> {
  const r = (await arrangerApi(page, "clipRect", id))!;
  return { x: r.x + r.width * fx, y: r.y + r.height * 0.7 };
}

test("double-tap makes a clip; a tap selects it: the action bar and its trim grips", async ({ page }) => {
  await launchMobile(page);
  const { id } = await midiClip(page);
  const touch = await touchscreen(page);
  await touch.tap(await onClip(page, id));
  await expect(actions(page)).toBeVisible();
  await expect(page.getByTestId("clip-handles")).toHaveCount(1);
  for (const name of ["split", "duplicate", "copy", "paste", "delete", "more"]) {
    const box = (await actions(page).getByRole("button", { name, exact: true }).boundingBox())!;
    expect(Math.min(box.width, box.height)).toBeGreaterThanOrEqual(44);
  }
  await shot(page, "clip-selected");
  // A tap on empty space deselects: the bar goes.
  const r = (await arrangerApi(page, "clipRect", id))!;
  await touch.tap({ x: r.x + r.width + 120, y: r.y + r.height / 2 });
  await expect(actions(page)).toHaveCount(0);
});

test("a drag on a selected clip moves it (snapped, badge above the finger); long-press then drag moves an unselected one; a plain drag on it pans", async ({
  page,
}) => {
  await launchMobile(page);
  const { id } = await midiClip(page);
  const touch = await touchscreen(page);
  const ppb = (await arrangementViewState(page)).pxPerBeat;
  // Select, then drag: it moves by 4 beats (snapped to the grid), the badge shows where.
  await touch.tap(await onClip(page, id));
  await expect(actions(page)).toBeVisible();
  const b = await onClip(page, id, 0.4);
  await touch.start([{ ...b, id: 1 }]);
  for (let i = 1; i <= 8; i++) {
    await touch.move([{ x: b.x + (4 * ppb * i) / 8, y: b.y, id: 1 }]);
    await touch.sleep(16);
  }
  await expect(page.getByTestId("drag-badge")).toBeVisible();
  await expect(page.getByTestId("drag-badge")).toHaveText("2.1.1");
  const badge = (await page.getByTestId("drag-badge").boundingBox())!;
  expect(badge.y + badge.height).toBeLessThan(b.y - 30);
  await expect(actions(page)).toHaveCount(0);
  await shot(page, "clip-drag");
  await touch.end();
  await expect.poll(async () => (await clip(page, id)).start).toBe(4);
  await expect(page.getByTestId("drag-badge")).toHaveCount(0);
  // Deselect (a tap on the empty lane selects the track), then long-press and drag: it moves back.
  const r = (await arrangerApi(page, "clipRect", id))!;
  await touch.tap({ x: r.x + r.width + 40, y: r.y + r.height / 2 });
  await expect(actions(page)).toHaveCount(0);
  const c = await onClip(page, id, 0.4);
  await touch.drag(c, { x: c.x - 4 * ppb, y: c.y }, { hold: 600 });
  await expect.poll(async () => (await clip(page, id)).start).toBe(0);
  // Unselected again: a plain drag over it pans, the clip stays.
  const r2 = (await arrangerApi(page, "clipRect", id))!;
  await touch.tap({ x: r2.x + r2.width + 40, y: r2.y + r2.height / 2 });
  const s0 = (await arrangementViewState(page)).scrollBeats;
  const d = await onClip(page, id, 0.4);
  await touch.drag(d, { x: d.x - 2 * ppb, y: d.y });
  expect((await clip(page, id)).start).toBe(0);
  expect((await arrangementViewState(page)).scrollBeats).toBeGreaterThan(s0);
});

test("trim with the wide handles; a second finger cancels a drag", async ({ page }) => {
  await launchMobile(page);
  const { id } = await midiClip(page);
  const touch = await touchscreen(page);
  const ppb = (await arrangementViewState(page)).pxPerBeat;
  await touch.tap(await onClip(page, id));
  // The end handle reaches outside the clip: grab 12 px past its right edge.
  const r = (await arrangerApi(page, "clipRect", id))!;
  const end = { x: r.x + r.width + 12, y: r.y + r.height / 2 };
  await touch.drag(end, { x: end.x + 2 * ppb, y: end.y });
  await expect.poll(async () => (await clip(page, id)).length).toBe(6);
  expect((await clip(page, id)).start).toBe(0);
  // The start handle, from just inside the clip.
  const r2 = (await arrangerApi(page, "clipRect", id))!;
  const start = { x: r2.x + 8, y: r2.y + r2.height / 2 };
  await touch.drag(start, { x: start.x + 2 * ppb, y: start.y });
  await expect.poll(async () => (await clip(page, id)).start).toBe(2);
  // A second finger mid-drag: the clip stays where it was.
  const before = await clip(page, id);
  const m = await onClip(page, id);
  await touch.start([{ ...m, id: 1 }]);
  for (let i = 1; i <= 6; i++) await touch.move([{ x: m.x + 5 * i, y: m.y, id: 1 }]);
  await touch.start([
    { x: m.x + 30, y: m.y, id: 1 },
    { x: m.x + 30, y: m.y + 120, id: 2 },
  ]);
  await touch.end();
  await page.waitForTimeout(200);
  expect(await clip(page, id)).toEqual(before);
});

test("the action bar: split at the tap, duplicate, copy and paste, delete", async ({ page }) => {
  await launchMobile(page);
  const { id } = await midiClip(page);
  const touch = await touchscreen(page);
  // A tap at the clip's middle selects it and puts the insert marker there: Split cuts there.
  await touch.tap(await onClip(page, id, 0.5));
  await actions(page).getByRole("button", { name: "split", exact: true }).tap();
  await expect
    .poll(async () => (await clips(page)).map((c) => [c.start, c.length]).sort())
    .toEqual([
      [0, 2],
      [2, 2],
    ]);
  // Duplicate the right part: a copy after it.
  const right = (await clips(page)).find((c) => c.start === 2)!;
  await touch.tap(await onClip(page, right.id, 0.5));
  await expect(actions(page)).toBeVisible();
  await actions(page).getByRole("button", { name: "duplicate", exact: true }).tap();
  await expect.poll(async () => (await clips(page)).length).toBe(3);
  // Copy the selection (the copy), then paste.
  await actions(page).getByRole("button", { name: "copy", exact: true }).tap();
  await expect(actions(page).getByRole("button", { name: "paste", exact: true })).toBeEnabled();
  await actions(page).getByRole("button", { name: "paste", exact: true }).tap();
  await expect.poll(async () => (await clips(page)).length).toBe(4);
  // Delete the selection.
  await actions(page).getByRole("button", { name: "delete", exact: true }).tap();
  await expect.poll(async () => (await clips(page)).length).toBe(3);
});

test("dragging a clip to the lanes' right edge scrolls the timeline along", async ({ page }) => {
  await launchMobile(page);
  const { id } = await midiClip(page);
  const touch = await touchscreen(page);
  await touch.tap(await onClip(page, id));
  const a = await onClip(page, id);
  const scroller = (await page.locator(".eth-arr__scroll").boundingBox())!;
  const s0 = (await arrangementViewState(page)).scrollBeats;
  await touch.start([{ ...a, id: 1 }]);
  for (let i = 1; i <= 10; i++) {
    await touch.move([{ x: a.x + ((scroller.x + scroller.width - 8 - a.x) * i) / 10, y: a.y, id: 1 }]);
    await touch.sleep(16);
  }
  await touch.sleep(800);
  await touch.end();
  const v = await arrangementViewState(page);
  expect(v.scrollBeats).toBeGreaterThan(s0 + 4);
  // The clip went along: it starts past where the view started scrolling.
  await expect.poll(async () => (await clip(page, id)).start).toBeGreaterThan(16);
});
