// Selection on touch screens (#522, no Shift or Ctrl): "select more" turns taps into toggles and
// a drag on empty space into a selection box; ◀ ▶ nudge the selection one grid step; long-press
// then drag on empty lane space selects a time range (its handles drag its ends), whose bar
// loops it; the clip sheet takes typed positions.
import { expect, test, type Page } from "@playwright/test";
import { arrangerApi, isClipSelected } from "./clips";
import { arrangementViewState, centerOf, IPHONE, launchMobile, shot, touchscreen, type Pt } from "./mobile";
import { createTrack } from "./ui";

test.use(IPHONE);

const clips = (page: Page) =>
  page.evaluate(() =>
    Object.values(
      (window as unknown as { __ether: { state(): { project: { clips: Record<string, { id: string; start: number }> } } } }).__ether.state().project.clips,
    ).map((c) => ({ id: c.id, start: c.start })),
  );
const actions = (page: Page) => page.getByTestId("clip-actions");

/** A MIDI track with one-bar clips made by double-taps on its lane at these beats. */
async function midiClips(page: Page, beats: number[]) {
  const track = await createTrack(page, "Midi");
  const touch = await touchscreen(page);
  const row = (await page.locator(`.eth-arr__scroll .eth-arr-header[data-track="${track.id}"]`).boundingBox())!;
  const lanes = (await page.getByTestId("arrangement-content").boundingBox())!;
  const ppb = (await arrangementViewState(page)).pxPerBeat;
  for (const [i, b] of beats.entries()) {
    const at = { x: lanes.x + row.width + b * ppb + 2, y: row.y + row.height * 0.7 };
    await touch.doubleTap(at);
    await expect.poll(async () => (await clips(page)).length).toBe(i + 1);
    await touch.sleep(400);
  }
  const all = (await clips(page)).sort((a, b) => a.start - b.start);
  return { touch, ppb, row, lanes, ids: all.map((c) => c.id) };
}

async function onClip(page: Page, id: string): Promise<Pt> {
  const r = (await arrangerApi(page, "clipRect", id))!;
  return { x: r.x + r.width / 2, y: r.y + r.height * 0.7 };
}

test("select more: taps toggle, a drag draws a box; done ends it", async ({ page }) => {
  await launchMobile(page);
  const { touch, ids, row, lanes } = await midiClips(page, [1, 9]);
  const [a, b] = ids as [string, string];
  await touch.tap(await onClip(page, a));
  await actions(page).getByRole("button", { name: "select more" }).tap();
  await touch.tap(await onClip(page, b));
  await expect.poll(() => isClipSelected(page, b)).toBe(true);
  expect(await isClipSelected(page, a)).toBe(true);
  await expect(actions(page).getByRole("button", { name: /^done/ })).toHaveText(/2/);
  await shot(page, "select-more");
  // A second tap takes it out.
  await touch.tap(await onClip(page, a));
  await expect.poll(() => isClipSelected(page, a)).toBe(false);
  // A drag from empty space (between the clips) back over the first: a box selecting it (a
  // plain box replaces the selection).
  const ppb = (await arrangementViewState(page)).pxPerBeat;
  await touch.drag({ x: lanes.x + row.width + 6 * ppb, y: row.y + 4 }, { x: lanes.x + row.width + 2 * ppb, y: row.y + row.height - 4 });
  await expect.poll(() => isClipSelected(page, a)).toBe(true);
  expect(await isClipSelected(page, b)).toBe(false);
  await touch.tap(await centerOf(actions(page).getByRole("button", { name: /^done/ })));
  await expect(actions(page).getByRole("button", { name: "select more" })).toBeVisible();
});

test("nudge: earlier / later move the selection by a grid step", async ({ page }) => {
  await launchMobile(page);
  const { touch, ids } = await midiClips(page, [5]);
  const [a] = ids as [string];
  const start = (await clips(page))[0]!.start;
  await touch.tap(await onClip(page, a));
  await actions(page).getByRole("button", { name: "later" }).tap();
  await expect.poll(async () => (await clips(page))[0]!.start).toBeGreaterThan(start);
  const later = (await clips(page))[0]!.start;
  await actions(page).getByRole("button", { name: "earlier" }).tap();
  await actions(page).getByRole("button", { name: "earlier" }).tap();
  await expect.poll(async () => (await clips(page))[0]!.start).toBeCloseTo(start - (later - start), 6);
});

test("long-press then drag on empty lane space: a time range, looped from its bar", async ({ page }) => {
  await launchMobile(page);
  const { touch, row, lanes, ppb } = await midiClips(page, [9]);
  const from = { x: lanes.x + row.width + 1 * ppb, y: row.y + row.height / 2 };
  await touch.drag(from, { x: from.x + 4 * ppb, y: from.y }, { hold: 600 });
  const bar = page.getByTestId("range-actions");
  await expect(bar).toBeVisible();
  await shot(page, "range");
  await bar.getByRole("button", { name: "loop it" }).tap();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const s = (
          window as unknown as {
            __ether: { state(): { project: { settings: { loop_enabled: boolean; loop_region: { start: number; end: number } | null } } } };
          }
        ).__ether.state().project.settings;
        return s.loop_enabled && s.loop_region ? s.loop_region.end - s.loop_region.start : 0;
      }),
    )
    .toBeGreaterThan(2);
});

test("the range's handles drag its ends", async ({ page }) => {
  await launchMobile(page);
  const { touch, row, lanes, ppb } = await midiClips(page, [9]);
  const from = { x: lanes.x + row.width + 1 * ppb, y: row.y + row.height / 2 };
  await touch.drag(from, { x: from.x + 4 * ppb, y: from.y }, { hold: 600 });
  const end = page.getByTestId("range-handle-end");
  await expect(end).toBeVisible();
  const before = (await end.boundingBox())!;
  // The handle's target is wider than drawn: grab it a little beside its line.
  await touch.drag({ x: before.x + before.width / 2 + 10, y: row.y + row.height / 2 }, { x: before.x + 4 * ppb, y: row.y + row.height / 2 });
  await expect.poll(async () => (await end.boundingBox())!.x).toBeGreaterThan(before.x + 2 * ppb);
  // The start handle, dragged right.
  const start = page.getByTestId("range-handle-start");
  const s0 = (await start.boundingBox())!;
  await touch.drag({ x: s0.x + s0.width / 2, y: row.y + row.height / 2 }, { x: s0.x + 3 * ppb, y: row.y + row.height / 2 });
  await expect.poll(async () => (await start.boundingBox())!.x).toBeGreaterThan(s0.x + ppb);
  expect((await start.boundingBox())!.x).toBeLessThan((await end.boundingBox())!.x);
  await expect(page.getByTestId("range-actions")).toBeVisible();
  await shot(page, "range-handles");
});

test("the clip sheet: start and length typed in, an edge nudged", async ({ page }) => {
  await launchMobile(page);
  const { touch, ids } = await midiClips(page, [5]);
  const [a] = ids as [string];
  await touch.tap(await onClip(page, a));
  await actions(page).getByRole("button", { name: "position" }).tap();
  const sheet = page.getByTestId("clip-sheet");
  await expect(sheet).toBeVisible();
  const clip = () =>
    page.evaluate(
      (id) =>
        (window as unknown as { __ether: { state(): { project: { clips: Record<string, { start: number; length: number }> } } } }).__ether.state().project
          .clips[id]!,
      a,
    );
  await sheet.getByLabel("clip start").fill("3.1.1");
  await sheet.getByLabel("clip start").press("Enter");
  await expect.poll(async () => (await clip()).start).toBe(8);
  await sheet.getByLabel("clip length").fill("0.2");
  await sheet.getByLabel("clip length").press("Enter");
  await expect.poll(async () => (await clip()).length).toBe(2);
  // Nonsense goes back to the value.
  await sheet.getByLabel("clip start").fill("banana");
  await sheet.getByLabel("clip start").press("Enter");
  await expect(sheet.getByLabel("clip start")).toHaveValue("3.1.1");
  await shot(page, "clip-sheet");
  await sheet.getByRole("button", { name: "end edge later" }).tap();
  await expect.poll(async () => (await clip()).length).toBeGreaterThan(2);
  expect((await clip()).start).toBe(8);
  await sheet.getByRole("button", { name: "start edge later" }).tap();
  await expect.poll(async () => (await clip()).start).toBeGreaterThan(8);
});
