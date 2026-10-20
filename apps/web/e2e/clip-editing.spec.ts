// Clip editing on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet): drag an audio clip's fade-in handle (SetFades), reverse
// it from the clip menu (badge), reverse a trimmed clip (its offset mirrored), then
// add, rename and delete an arrangement marker.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { arrangerApi, clipDecor, clipMenu, clipPart, expectProjectRenderer, selectClip } from "./clips";
import { installProbe, send } from "./perfProject";
import { launch, newProject, expandFolder } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const audioOf = async (page: Page, clip: string) => {
  const c: Clip = (await doc(page)).clips[clip]!;
  if (c.content.type !== "Audio") throw new Error("not audio");
  return c.content;
};

const markers = async (page: Page) => Object.values((await doc(page)).markers).sort((a, b) => a.position - b.position);

test("clip editing: fade drag, reverse, markers", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await newProject(page, `Clip editing ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);

  // --- Audio track + library loop ----------------------------------------------------------
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  // The sample browser is a pane opened from the rail; pinned, it doesn't cover the lanes.
  await page.getByRole("button", { name: "library", exact: true }).click();
  await page.getByRole("button", { name: "pin the library panel" }).click();
  await expandFolder(page, "factory");
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await expectProjectRenderer(page, test.info().project.name);
  // --- Fade-in drag: the handle follows the pointer (px → beats at the current zoom) ------
  // The handles and hits come from the arranger test API.
  await selectClip(page, clip.id);
  let box = await clipPart(page, clip.id, "fade-in");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  box = await clipPart(page, clip.id, "fade-in");

  // Near the fade handle, the fade wins over the clip's edge resize strip; lower down the
  // edge, the resize strip is still there.
  const clipBox = await clipPart(page, clip.id, "box");
  const hitAt = async (x: number, y: number) => (await arrangerApi(page, "hitTest", x, y))?.part ?? "none";
  expect(await hitAt(clipBox.x + 4, box.y + box.height / 2)).toBe("fade-in");
  expect(await hitAt(clipBox.x + 4, clipBox.y + clipBox.height - 2)).toBe("resize-start");
  // Over the clip header (above the body's top edge), the fade handles never take the pointer.
  const title = await clipPart(page, clip.id, "header");
  for (const x of [box.x + box.width / 2, box.x + box.width + 6]) {
    expect(await hitAt(x, title.y + title.height / 2)).not.toBe("fade-in");
    expect(await hitAt(x, title.y + title.height - 1)).not.toBe("fade-in");
  }
  const clipWidth = clipBox.width;
  expect(clipWidth).toBeGreaterThan(0);
  const lengthBeats = clip.length;
  const dx = 60;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + dx / 2, box.y + box.height / 2, { steps: 3 });
  await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2, { steps: 3 });
  await page.mouse.up();
  // The handle starts at the clip-start inset (fade 0), so the fade ends up a bit less than dx.
  const expected = (dx / clipWidth) * lengthBeats;
  await expect.poll(async () => (await audioOf(page, clip.id)).fade_in).toBeGreaterThan(expected * 0.5);
  expect((await audioOf(page, clip.id)).fade_in).toBeLessThanOrEqual(expected + 1e-6);
  await expect.poll(async () => (await clipDecor(page, clip.id))?.lines).toBe(1);

  // --- Reverse from the clip context menu ----------------------------------------------------
  await clipMenu(page, clip.id);
  await page.getByRole("menuitem", { name: "reverse", exact: true }).click();
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(true);
  await expect.poll(async () => (await clipDecor(page, clip.id))?.badges).toEqual(["reversed"]);
  // R toggles it back (and again) on the selected clip.
  await selectClip(page, clip.id);
  await page.keyboard.press("r");
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(false);
  await expect.poll(async () => (await clipDecor(page, clip.id))?.badges).toEqual([]);
  await page.keyboard.press("r");
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(true);

  // --- A trimmed clip reversed plays the same material backwards ----------------------
  // Its offset is mirrored over the file (K = the file's length in beats): content 1 .. 3
  // becomes K - 3 .. K - 1 of the reversed file; reversing back restores it.
  await page.keyboard.press("r");
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(false);
  await installProbe(page);
  const p0 = await doc(page);
  const media = p0.media[(await audioOf(page, clip.id)).media]!;
  const bpm = Object.values(p0.tempo_points).find((t) => t.time === 0)!.bpm;
  const fileBeats = ((media.frames / media.sample_rate) * bpm) / 60;
  const start = p0.clips[clip.id]!.start;
  await send(page, { domain: "Clip", command: { type: "SetBounds", id: clip.id, start, length: 2, offset: 1 } });
  await selectClip(page, clip.id);
  await page.keyboard.press("r");
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(true);
  expect((await doc(page)).clips[clip.id]!.offset).toBeCloseTo(fileBeats - 3, 6);
  expect((await doc(page)).clips[clip.id]!.length).toBeCloseTo(2, 6);
  await page.keyboard.press("r");
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(false);
  expect((await doc(page)).clips[clip.id]!.offset).toBeCloseTo(1, 6);

  // --- Markers: add (double-click the lane), rename, delete ----------------------------------
  const lane = page.getByTestId("marker-lane-area");
  await lane.dblclick({ position: { x: 100, y: 5 } });
  await expect.poll(async () => (await markers(page)).length).toBe(1);
  const marker = page.getByTestId("marker");
  await expect(marker).toHaveText("marker 1");
  await marker.dblclick();
  const name = page.getByLabel("marker name");
  await name.fill("Chorus");
  await name.press("Enter");
  await expect.poll(async () => (await markers(page))[0]?.name).toBe("Chorus");
  await marker.click({ button: "right" });
  await page.getByRole("menuitem", { name: "delete marker" }).click();
  await expect.poll(async () => (await markers(page)).length).toBe(0);

  expect(errors).toEqual([]);
});
