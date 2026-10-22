// Owner request: a click on the empty space of a track's lane selects the track and shows it
// in the inspector, like a click on its header. A drag there is still a selection box and a
// double-click still creates a MIDI clip. Runs on WebGL (`webgl` project) and on the Canvas2D
// fallback (`canvas2d` project: listed in ARRANGER_SPECS).
//
// `E2E_SHOTS=<dir>` saves the PR screenshots.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { expectProjectRenderer } from "./clips";
import { createTrack, launch, showInspector } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

const shot = async (page: Page, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

/** A point in a lane, `fx` of its width in (client px). */
async function lanePoint(page: Page, track: string, fx: number): Promise<{ x: number; y: number }> {
  const box = (await page.locator(`[data-lane="${track}"]`).boundingBox())!;
  return { x: box.x + box.width * fx, y: box.y + box.height / 2 };
}

test("a click on a lane's empty space selects the track (the inspector folds to its bar and shows it when expanded)", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Lane click ${Date.now()}`);
  await expectProjectRenderer(page, test.info().project.name);
  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");
  const clipsOn = async (t: string) => Object.values((await doc(page)).clips).filter((c) => c.track === t).length;
  const inspector = page.getByTestId("inspector");
  const trackName = inspector.getByRole("textbox", { name: "track name" });
  const header = (name: string) => page.getByRole("group", { name: `${name} track` });

  // A clip on A (double-click on its empty lane still creates one).
  await page.locator(`[data-lane="${a.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(() => clipsOn(a.id)).toBe(1);

  // --- Click on B's empty space: B is selected; the inspector folds to its bar (a click on the
  // lanes' empty space collapses it) and shows B when expanded --------------------------------------
  const inspectorBar = page.getByRole("button", { name: "expand the inspector panel" });
  let p = await lanePoint(page, b.id, 0.6);
  await page.mouse.click(p.x, p.y);
  await expect(header(b.name)).toHaveClass(/eth-arr-header--selected/);
  await expect(header(a.name)).not.toHaveClass(/eth-arr-header--selected/);
  await expect(inspectorBar).toBeVisible();
  await showInspector(page);
  await expect(trackName).toHaveValue(b.name);
  await shot(page, "lane-click-selects-track");

  // --- And on A's empty space (right of its clip): A ------------------------------------------------
  p = await lanePoint(page, a.id, 0.6);
  await page.mouse.click(p.x, p.y);
  await expect(header(a.name)).toHaveClass(/eth-arr-header--selected/);
  await expect(inspectorBar).toBeVisible();
  await showInspector(page);
  await expect(trackName).toHaveValue(a.name);
  await expect(header(b.name)).not.toHaveClass(/eth-arr-header--selected/);

  // --- Again on A's empty space, with the inspector expanded on A: it stays expanded (#221) ---------
  p = await lanePoint(page, a.id, 0.45); // (left of the expanded inspector, which floats over the lanes)
  await page.mouse.click(p.x, p.y);
  await page.waitForTimeout(500); // past the collapse animation: asserts it stayed, not that it has not folded yet
  await expect(trackName).toBeVisible();
  await expect(trackName).toHaveValue(a.name);
  await expect(inspectorBar).toBeHidden();

  // --- Cmd/Ctrl-click adds B, like on the header -----------------------------------------------------
  p = await lanePoint(page, b.id, 0.6);
  await page.keyboard.down("ControlOrMeta");
  await page.mouse.click(p.x, p.y);
  await page.keyboard.up("ControlOrMeta");
  await expect(header(a.name)).toHaveClass(/eth-arr-header--selected/);
  await expect(header(b.name)).toHaveClass(/eth-arr-header--selected/);

  // --- A drag over empty space is still a selection box (no track change) ---------------------------
  p = await lanePoint(page, a.id, 0.5);
  const q = await lanePoint(page, b.id, 0.7);
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.move((p.x + q.x) / 2, (p.y + q.y) / 2, { steps: 4 });
  await page.mouse.move(q.x, q.y, { steps: 4 });
  await expect(page.getByTestId("marquee")).toBeVisible();
  await shot(page, "lane-drag-marquee");
  await page.mouse.up();
  await expect(page.getByTestId("marquee")).toHaveCount(0);
  await expect(header(a.name)).toHaveClass(/eth-arr-header--selected/);
  await expect(header(b.name)).toHaveClass(/eth-arr-header--selected/);

  // --- A click on the clip selects the clip (the inspector shows it, not the track) ------------------
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === a.id)!;
  const clipBox = await page.evaluate(
    (id) =>
      (
        window as unknown as { __ether: { arrangement: { clipRect(id: string): { x: number; y: number; width: number; height: number } | null } } }
      ).__ether.arrangement.clipRect(id),
    clip.id,
  );
  await page.mouse.click(clipBox!.x + clipBox!.width / 2, clipBox!.y + clipBox!.height * 0.75);
  // (A click on a clip never expands a pane.)
  await showInspector(page);
  await expect(inspector.getByRole("textbox", { name: "clip name" })).toBeVisible();
  await expect(header(a.name)).not.toHaveClass(/eth-arr-header--selected/);

  // --- With the clip shown, A's empty space keeps the inspector expanded, now on A (the click
  // selects the track, the clip selection clears) ---------------------------------------------------
  p = await lanePoint(page, a.id, 0.45);
  await page.mouse.click(p.x, p.y);
  await page.waitForTimeout(500); // past the collapse animation
  await expect(inspectorBar).toBeHidden();
  await expect(trackName).toHaveValue(a.name);
  await expect(header(a.name)).toHaveClass(/eth-arr-header--selected/);

  // --- With the clip shown, another track's empty space still collapses it -------------------------
  await page.mouse.click(clipBox!.x + clipBox!.width / 2, clipBox!.y + clipBox!.height * 0.75);
  await showInspector(page);
  await expect(inspector.getByRole("textbox", { name: "clip name" })).toBeVisible();
  p = await lanePoint(page, b.id, 0.45);
  await page.mouse.click(p.x, p.y);
  await expect(inspectorBar).toBeVisible();

  // --- Double-click on B's empty space still creates a MIDI clip there -------------------------------
  p = await lanePoint(page, b.id, 0.3);
  await page.mouse.dblclick(p.x, p.y);
  await expect.poll(() => clipsOn(b.id)).toBe(1);

  expect(errors).toEqual([]);
});
