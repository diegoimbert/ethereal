// base-144 on the web build, through the UI, against the real engine: transposing an
// unwarped audio clip in the Inspector resizes it in the arranger (an octave up plays twice
// as fast: half as long), and a clip that gets longer stops at the next clip on its track.
//
// Screenshots for the PR (skipped unless `TRANSPOSE_SHOTS=<dir>`; `TRANSPOSE_THEME=light`
// for the light theme).
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { arrangerApi, selectClip } from "./clips";
import { expandFolder, launch, openLibrary, setNumberField } from "./ui";

const shots = process.env.TRANSPOSE_SHOTS;
const theme = process.env.TRANSPOSE_THEME ?? "dark";

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const clip = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;
const lengthOf = async (page: Page, id: string): Promise<number> => (await clip(page, id)).length;

async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  await page.mouse.move(0, 0);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.screenshot({ path: `${shots}/${theme}-${name}.png` });
}

test("transposing an audio clip resizes it, trimmed at the next clip", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page, `Transpose ${Date.now()}`);

  // --- Audio track, a sample, and a copy right after it ----------------------------------
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const a = Object.values((await doc(page)).clips)[0]!;
  expect(a.content.type === "Audio" && a.content.warp.enabled).toBe(false);
  const full = a.length;
  await selectClip(page, a.id);
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(2);
  const b = Object.values((await doc(page)).clips).find((c) => c.id !== a.id)!;
  expect(b.start).toBeCloseTo(a.start + full, 9);

  const transpose = page.getByTestId("inspector").getByRole("spinbutton", { name: "clip transpose" });
  await selectClip(page, a.id);
  await expect(transpose).toBeVisible();

  // --- An octave down: twice as long, but trimmed where the copy starts ----------------
  await setNumberField(transpose, -12);
  await expect.poll(async () => (await clip(page, a.id)).content).toMatchObject({ transpose: -12 });
  const down = await clip(page, a.id);
  expect(down.start + down.length).toBeCloseTo(b.start, 9);
  expect(await clip(page, b.id)).toMatchObject({ start: b.start, length: b.length, offset: b.offset });
  await expect.poll(() => arrangerApi(page, "visibleClipIds")).toContain(a.id);
  await shot(page, "1-octave-down-trimmed-at-next-clip");

  // --- Back up an octave: twice as fast, half as long (a gap before the copy) -----------
  await setNumberField(transpose, 0);
  await expect.poll(() => lengthOf(page, a.id)).toBeCloseTo(down.length / 2, 9);
  await shot(page, "2-octave-up-half-length");

  // --- One undo step restores the transpose and the length exactly ---------------------
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(async () => (await clip(page, a.id)).content).toMatchObject({ transpose: -12 });
  expect(await lengthOf(page, a.id)).toBeCloseTo(down.length, 9);

  expect(errors).toEqual([]);
});
