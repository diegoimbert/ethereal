// #791: return tracks reorder by dragging their headers among the returns. Add two returns,
// add a third, drag it between the other two: the order is right, and each send still goes
// to its return (the inspector's send knobs follow the new order).
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { createTrack, launch, newProject, playButton, selectTrack } from "./ui";

test.use({ viewport: { width: 1600, height: 900 } });

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project } } }).__ether.state().project);

const ordered = (p: Project, kind: Track["kind"]): Track[] =>
  Object.values(p.tracks)
    .filter((t) => t.kind === kind && t.parent === null)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));

async function addReturn(page: Page): Promise<Track> {
  const before = new Set(Object.keys((await project(page)).tracks));
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create return track" }).click();
  let added: Track | undefined;
  await expect
    .poll(async () => {
      added = ordered(await project(page), "Return").find((t) => !before.has(t.id));
      return added !== undefined;
    })
    .toBe(true);
  return added!;
}

test("a new return drags between two others; the sends keep their returns", async ({ page }) => {
  test.setTimeout(90_000);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Return reorder ${Date.now()}`);

  const audio = await createTrack(page, "Audio");
  const a = await addReturn(page);
  const b = await addReturn(page);
  // The third one, added with b selected: right after it (at the end).
  const c = await addReturn(page);
  expect(ordered(await project(page), "Return").map((t) => t.id)).toEqual([a.id, b.id, c.id]);

  // The audio track sends to each return (its send knobs, one step up each).
  await selectTrack(page, audio.name);
  const sends = page.getByTestId("inspector").getByLabel("sends");
  for (const r of [a, b, c]) {
    await sends.getByRole("slider", { name: `send ${r.name}` }).focus();
    await page.keyboard.press("ArrowUp");
  }
  await expect.poll(async () => Object.values((await project(page)).sends).filter((s) => s.from === audio.id).length).toBe(3);
  const sendTo = new Map(Object.values((await project(page)).sends).map((s) => [s.id, s.to]));

  // Drag c's header to the gap between a and b.
  const header = (t: Track) => page.getByRole("group", { name: `${t.name} track` });
  const from = (await header(c).boundingBox())!;
  const gap = (await header(b).boundingBox())!;
  await page.mouse.move(from.x + 8, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + 8, from.y + from.height / 2 - 10, { steps: 3 });
  await page.mouse.move(from.x + 8, gap.y + 4, { steps: 6 });
  await expect(page.getByTestId("track-drop-line")).toBeVisible();
  // Over the regular tracks: the not-allowed zone, no line.
  const regular = (await header(audio).boundingBox())!;
  await page.mouse.move(from.x + 8, regular.y + regular.height / 2, { steps: 6 });
  await expect(page.getByTestId("track-drop-blocked")).toBeVisible();
  await expect(page.getByTestId("track-drop-line")).toBeHidden();
  // Back to the gap, and drop.
  await page.mouse.move(from.x + 8, gap.y + 4, { steps: 6 });
  await expect(page.getByTestId("track-drop-line")).toBeVisible();
  await page.mouse.up();

  await expect.poll(async () => ordered(await project(page), "Return").map((t) => t.id)).toEqual([a.id, c.id, b.id]);
  const p = await project(page);
  for (const [id, to] of sendTo) expect(p.sends[id]?.to).toBe(to);
  // The regular track stays above the returns.
  expect(ordered(p, "Audio").map((t) => t.id)).toEqual([audio.id]);
  // The inspector's send knobs follow the new order.
  await selectTrack(page, audio.name);
  await expect
    .poll(() => sends.getByRole("slider").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label"))))
    .toEqual([`send ${a.name}`, `send ${c.name}`, `send ${b.name}`]);

  // One undo step.
  await header(a).click({ position: { x: 6, y: 10 } });
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => ordered(await project(page), "Return").map((t) => t.id)).toEqual([a.id, b.id, c.id]);
});
