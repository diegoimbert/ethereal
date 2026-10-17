// Owner request: Ctrl/Cmd+A with a track selected (not a clip) selects every track, not every
// clip. With a clip selected it still selects every clip.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

const selectedClips = (page: Page) => page.evaluate(() => (window as unknown as { __ether: { selectedClips(): string[] } }).__ether.selectedClips());

test("Ctrl+A selects every track after a header click, every clip after a clip click", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Select all tracks ${Date.now()}`);
  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  const c = await createTrack(page, "Audio");
  const header = (name: string) => page.getByRole("group", { name: `${name} track` });
  const clipsOf = async () => Object.values((await doc(page)).clips).filter((x) => !x.lane);

  // A clip on A and on B (a double-click on an empty lane creates one).
  await page.locator(`[data-lane="${a.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await page.locator(`[data-lane="${b.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => (await clipsOf()).length).toBe(2);

  // --- Header click, Ctrl+A: every track but the master, no clip -------------------------------
  await header(b.name).click();
  await page.keyboard.press("ControlOrMeta+a");
  const nonMaster = Object.values((await doc(page)).tracks).filter((t) => t.kind !== "Master");
  for (const t of [a, b, c]) await expect(header(t.name)).toHaveClass(/eth-arr-header--selected/);
  await expect(page.locator(".eth-arr-header--selected")).toHaveCount(nonMaster.length);
  expect(await selectedClips(page)).toEqual([]);

  // --- Clip click, Ctrl+A: every clip, no track ------------------------------------------------
  const clip = (await clipsOf()).find((x) => x.track === a.id)!;
  const box = await page.evaluate(
    (id) =>
      (
        window as unknown as { __ether: { arrangement: { clipRect(id: string): { x: number; y: number; width: number; height: number } | null } } }
      ).__ether.arrangement.clipRect(id),
    clip.id,
  );
  await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height * 0.75);
  await expect.poll(() => selectedClips(page)).toEqual([clip.id]);
  await page.keyboard.press("ControlOrMeta+a");
  const all = (await clipsOf()).map((x) => x.id).sort();
  await expect.poll(async () => (await selectedClips(page)).sort()).toEqual(all);
  await expect(page.locator(".eth-arr-header--selected")).toHaveCount(0);

  expect(errors).toEqual([]);
});
