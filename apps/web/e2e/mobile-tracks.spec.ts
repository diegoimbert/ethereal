// Tracks on touch screens: the headers' mute / solo / arm are always shown (no hover)
// and work by tap; a tap on the selected header opens the track sheet (volume, pan, rename,
// move, the track menu's actions, the device chain read-only), which the system back closes;
// long-press opens the track menu at touch size; long-press then drag reorders; Add track by tap.
import { expect, test, type Page } from "@playwright/test";
import { centerOf, IPHONE, launchMobile, settled, shot, touchscreen } from "./mobile";

test.use(IPHONE);

interface TrackInfo {
  id: string;
  name: string;
  kind: string;
  mixer: { mute: boolean; solo: boolean; volume: number; pan: number };
}
const tracks = (page: Page) =>
  page.evaluate(() => {
    const p = (window as unknown as { __ether: { state(): { project: { tracks: Record<string, TrackInfo> } | null } } }).__ether.state().project;
    return Object.values(p?.tracks ?? {}).filter((t) => t.kind !== "Master");
  });
/** Track ids in header order. */
const headerOrder = (page: Page) =>
  page.locator(".eth-arr__scroll .eth-arr-header[data-track]").evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.track!));
const header = (page: Page, id: string) => page.locator(`.eth-arr-header[data-track="${id}"]`);

/** Add a track by touch: Add track, then the kind. */
async function addTrack(page: Page, kind: "audio" | "MIDI"): Promise<TrackInfo> {
  const before = new Set((await tracks(page)).map((t) => t.id));
  await page.getByRole("button", { name: /^add track$/i }).tap();
  await page.getByRole("menuitem", { name: `create ${kind} track` }).tap();
  let created: TrackInfo | undefined;
  await expect
    .poll(async () => {
      created = (await tracks(page)).find((t) => !before.has(t.id));
      return !!created;
    })
    .toBe(true);
  if (kind === "MIDI") {
    const picker = page.getByRole("dialog", { name: /choose an instrument/ });
    await expect(picker).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(picker).toBeHidden();
  }
  return created!;
}

test("header buttons are shown without hover and work by tap", async ({ page }) => {
  await launchMobile(page);
  const a = await addTrack(page, "audio");
  const h = header(page, a.id);
  for (const name of [`mute ${a.name}`, `solo ${a.name}`, `arm ${a.name}`]) {
    const b = h.getByRole("button", { name });
    await expect(b).toBeVisible();
    expect(await b.evaluate((el) => getComputedStyle(el).opacity)).toBe("1");
    const box = (await b.boundingBox())!;
    expect(Math.min(box.width, box.height)).toBeGreaterThanOrEqual(34);
  }
  const touch = await touchscreen(page);
  await touch.tap(await centerOf(h.getByRole("button", { name: `mute ${a.name}` })));
  await expect.poll(async () => (await tracks(page))[0]!.mixer.mute).toBe(true);
  await touch.tap(await centerOf(h.getByRole("button", { name: `solo ${a.name}` })));
  await expect.poll(async () => (await tracks(page))[0]!.mixer.solo).toBe(true);
  await shot(page, "track-headers");
});

test("the track sheet: open by tapping the selected header; volume, rename, move, delete; back closes it", async ({ page }) => {
  await launchMobile(page);
  const a = await addTrack(page, "audio");
  const b = await addTrack(page, "MIDI");
  const touch = await touchscreen(page);
  // A tap on another track's header selects it; a tap on the selected one opens its sheet.
  await touch.tap(await centerOf(header(page, a.id).locator(".eth-arr-header__name")));
  await expect(header(page, a.id)).toHaveClass(/eth-arr-header--selected/);
  await page.waitForTimeout(350); // not a double-tap
  const name = await centerOf(header(page, b.id).locator(".eth-arr-header__name"));
  await touch.tap(name);
  await expect(header(page, b.id)).toHaveClass(/eth-arr-header--selected/);
  await expect(page.getByRole("dialog", { name: "track" })).toHaveCount(0);
  await page.waitForTimeout(350); // not a double-tap
  await touch.tap(name);
  const sheet = page.getByRole("dialog", { name: "track" });
  await expect(sheet).toBeVisible();
  // The device chain, read-only: the MIDI track's instrument, on.
  await expect(sheet.locator(".eth-track-sheet__device")).toHaveCount(1);
  await expect(sheet.locator(".eth-track-sheet__device")).toContainText("on");
  await expect(sheet.locator(".eth-dev, .eth-device")).toHaveCount(0);
  await shot(page, "track-sheet");
  // Volume: a drag on the sheet's fader (relative, no jump).
  await settled(page);
  const fader = sheet.getByRole("slider", { name: /volume/i });
  const f = (await fader.boundingBox())!;
  await touch.drag({ x: f.x + f.width * 0.5, y: f.y + f.height / 2 }, { x: f.x + f.width * 0.2, y: f.y + f.height / 2 }, { rest: 0 });
  await expect.poll(async () => (await tracks(page)).find((t) => t.id === b.id)!.mixer.volume).toBeLessThan(-3);
  // Rename.
  const field = sheet.getByRole("textbox", { name: "track name" });
  await field.fill("Lead");
  await field.press("Enter");
  await expect.poll(async () => (await tracks(page)).find((t) => t.id === b.id)!.name).toBe("Lead");
  // Move up: now first.
  expect(await headerOrder(page)).toEqual([a.id, b.id]);
  await sheet.getByRole("button", { name: "move up" }).tap();
  await expect.poll(() => headerOrder(page)).toEqual([b.id, a.id]);
  await expect(sheet.getByRole("button", { name: "move up" })).toBeDisabled();
  // The system back closes the sheet, the project stays.
  const url = page.url();
  await page.goBack();
  await expect(sheet).toHaveCount(0);
  expect(page.url()).toBe(url);
  await expect(page.getByTestId("mobile-shell")).toBeVisible();
  // Delete from the sheet: the track goes, the sheet closes.
  await touch.tap(await centerOf(header(page, b.id).locator(".eth-arr-header__name")));
  await expect(sheet).toBeVisible();
  await sheet.getByRole("button", { name: "delete track" }).tap();
  await expect.poll(async () => (await tracks(page)).map((t) => t.id)).toEqual([a.id]);
  await expect(sheet).toHaveCount(0);
});

test("long-press: the track menu at touch size; long-press then drag reorders", async ({ page }) => {
  await launchMobile(page);
  const a = await addTrack(page, "audio");
  const b = await addTrack(page, "audio");
  const touch = await touchscreen(page);
  await touch.longPress(await centerOf(header(page, a.id).locator(".eth-arr-header__name")));
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  const item = menu.getByRole("menuitem", { name: /duplicate track/ });
  // Its layout height (the menu scales in as it opens).
  expect(await item.evaluate((el) => (el as HTMLElement).offsetHeight)).toBeGreaterThanOrEqual(44);
  await shot(page, "track-menu");
  await item.tap();
  await expect.poll(async () => (await tracks(page)).length).toBe(3);
  // Reorder: hold the first header, then drag it below the last.
  const order = await headerOrder(page);
  const first = order[0]!;
  const from = await centerOf(header(page, first).locator(".eth-arr-header__name"));
  const last = (await header(page, order[2]!).boundingBox())!;
  await touch.drag(from, { x: from.x, y: last.y + last.height - 4 }, { hold: 600, steps: 16, ms: 400 });
  await expect.poll(async () => (await headerOrder(page)).at(-1)).toBe(first);
  expect(await headerOrder(page)).toHaveLength(3);
  void b;
});
