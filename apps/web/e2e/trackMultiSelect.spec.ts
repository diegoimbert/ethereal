// Track multi-selection on the web build. Select two tracks (click, then
// shift-click their headers): ⌘D duplicates both (with their clips) as one undo step, and
// the right-click menu acts on the whole selection ("Delete 2 Tracks").
//
// `TRACK_SELECT_SHOTS=<dir>` also saves the multi-track menu screenshot for the PR.
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { createTrack, launch } from "./ui";

const shots = process.env.TRACK_SELECT_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const content = async (page: Page) => Object.values((await doc(page)).tracks).filter((t) => t.kind === "Midi");
const clipCount = async (page: Page) => Object.values((await doc(page)).clips).length;

const header = (page: Page, t: Track) => page.getByRole("group", { name: `${t.name} track` }).first();

/** A header click at its left edge (the middle is the mute toggle). */
async function clickHeader(page: Page, t: Track, modifiers: Array<"Shift" | "ControlOrMeta"> = []): Promise<void> {
  await header(page, t).click({ position: { x: 6, y: 10 }, modifiers });
}

/** A MIDI clip on `t` (double-click in its lane). */
async function addClip(page: Page, t: Track): Promise<void> {
  const before = await clipCount(page);
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(() => clipCount(page)).toBe(before + 1);
}

test("Cmd+D duplicates the selected tracks with their clips; the menu acts on the whole selection", async ({ page }) => {
  await launch(page);
  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  await addClip(page, a);
  await addClip(page, b);

  await clickHeader(page, a);
  await clickHeader(page, b, ["Shift"]);
  await expect(page.locator(".eth-arr-header--selected")).toHaveCount(2);

  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(async () => (await content(page)).length).toBe(4);
  await expect.poll(() => clipCount(page)).toBe(4);
  // The copies are the selection.
  const copies = (await content(page)).filter((t) => t.id !== a.id && t.id !== b.id).map((t) => t.id);
  await expect(page.locator(".eth-arr-header--selected")).toHaveCount(2);
  for (const id of copies) await expect(page.locator(`.eth-arr-row[data-track="${id}"] .eth-arr-header--selected`)).toHaveCount(1);

  // One undo step.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await content(page)).length).toBe(2);
  await expect.poll(() => clipCount(page)).toBe(2);

  // Right-click on a selected track: the menu covers both.
  await clickHeader(page, a);
  await clickHeader(page, b, ["Shift"]);
  await header(page, b).click({ button: "right", position: { x: 6, y: 10 } });
  await expect(page.getByRole("menuitem", { name: /duplicate 2 tracks/ })).toBeVisible();
  if (shots) {
    await page.waitForTimeout(300); // the menu fades in
    await page.screenshot({ path: `${shots}/track-multi-menu.png` });
  }
  await page.getByRole("menuitem", { name: /delete 2 tracks/ }).click();
  await expect.poll(async () => (await content(page)).length).toBe(0);

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await content(page)).length).toBe(2);
});
