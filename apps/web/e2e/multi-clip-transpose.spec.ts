// On the web build, through the UI, against the real engine: with two audio clips
// selected, the Inspector's transpose applies to both (one undo step reverts both). Their
// values differ first, so the field shows mixed ("—"); an arrow step moves each by +1
// (differences kept); a typed value sets both. Each clip still resizes.
//
// Screenshots for the PR (skipped unless `MULTI_TRANSPOSE_SHOTS=<dir>`).
import { expect, test, type Page } from "@playwright/test";
import type { AudioContent, Clip, Project } from "@/generated";
import { selectClip } from "./clips";
import { expandFolder, launch, openLibrary, setNumberField } from "./ui";

const shots = process.env.MULTI_TRANSPOSE_SHOTS;

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const clip = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;
const transposes = async (page: Page, ids: string[]): Promise<number[]> =>
  Promise.all(ids.map(async (id) => ((await clip(page, id)).content as AudioContent).transpose));

async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  await page.mouse.move(0, 0);
  await page.screenshot({ path: `${shots}/${name}.png` });
}

test("transpose applies to every selected audio clip, one undo step", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await launch(page, `Multi transpose ${Date.now()}`);

  // --- An audio track with a sample and a copy right after it -----------------------------
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const track = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  await page
    .getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true })
    .dragTo(page.locator(`[data-lane="${track.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const a = Object.values((await doc(page)).clips)[0]!.id;
  await selectClip(page, a);
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(2);
  const b = Object.values((await doc(page)).clips).find((c) => c.id !== a)!.id;
  const ids = [a, b];
  const transpose = page.getByTestId("inspector").getByRole("spinbutton", { name: "clip transpose" });

  // --- B alone at +2 (the two values now differ) ------------------------------------------
  await selectClip(page, b);
  await setNumberField(transpose, 2);
  await expect.poll(() => transposes(page, ids)).toEqual([0, 2]);

  // --- Both selected: mixed --------------------------------------------------------------
  await selectClip(page, a);
  await selectClip(page, b, ["Shift"]);
  await expect(page.getByTestId("inspector").getByText("2 clips")).toBeVisible();
  await expect(transpose).toHaveValue("—");
  await shot(page, "1-two-clips-mixed-transpose");
  const lengths = await Promise.all(ids.map(async (id) => (await clip(page, id)).length));

  // --- Typed: both get it, each resizes; one undo reverts both ---------------------------
  await setNumberField(transpose, 7);
  await expect.poll(() => transposes(page, ids)).toEqual([7, 7]);
  await expect(transpose).toHaveValue("7");
  expect((await clip(page, a)).length).toBeCloseTo(lengths[0]! / 2 ** (7 / 12), 6);
  expect((await clip(page, b)).length).toBeCloseTo(lengths[1]! / 2 ** (5 / 12), 6);
  await shot(page, "2-both-transposed");
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(() => transposes(page, ids)).toEqual([0, 2]);
  expect((await clip(page, a)).length).toBeCloseTo(lengths[0]!, 9);
  expect((await clip(page, b)).length).toBeCloseTo(lengths[1]!, 9);

  // --- An arrow step: each +1 (differences kept), one undo -------------------------------
  await transpose.focus();
  await transpose.press("ArrowUp");
  await expect.poll(() => transposes(page, ids)).toEqual([1, 3]);
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(() => transposes(page, ids)).toEqual([0, 2]);

  expect(errors).toEqual([]);
});
