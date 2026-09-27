// Clip editing on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet): drag an audio clip's fade-in handle (SetFades), reverse
// it from the clip menu (badge), then add, rename and delete an arrangement marker.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { selectClip } from "./clips";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

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

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByLabel("New project name").fill(`Clip editing ${Date.now()}`);
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "New" }).click();
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);

  // --- Audio track + library loop ----------------------------------------------------------
  await page.getByRole("button", { name: "+ Audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  await page.getByRole("tablist", { name: "Locations" }).getByRole("tab", { name: "Browser library" }).click();
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const sample = page.getByRole("button", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  const clipEl = page.locator(`[data-clip-id="${clip.id}"]`);

  // --- Fade-in drag: the handle follows the pointer (px → beats at the current zoom) ------
  await selectClip(page, clip.id);
  const handle = clipEl.getByTestId("fade-in-handle");
  await handle.hover();
  const box = (await handle.boundingBox())!;
  const clipWidth = await page.evaluate(() => {
    const el = document.querySelector<HTMLElement>(".eth-clip");
    return el ? el.getBoundingClientRect().width : 0;
  });
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
  await expect(clipEl.locator(".eth-clip-fades__line")).toHaveCount(1);

  // --- Reverse from the clip context menu ----------------------------------------------------
  await clipEl.locator(".eth-clip__title").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Reverse", exact: true }).click();
  await expect.poll(async () => (await audioOf(page, clip.id)).reversed).toBe(true);
  await expect(clipEl.getByTestId("clip-reversed")).toBeVisible();

  // --- Markers: add (double-click the lane), rename, delete ----------------------------------
  const lane = page.getByTestId("marker-lane-area");
  await lane.dblclick({ position: { x: 100, y: 5 } });
  await expect.poll(async () => (await markers(page)).length).toBe(1);
  const marker = page.getByTestId("marker");
  await expect(marker).toHaveText("Marker 1");
  await marker.dblclick();
  const name = page.getByLabel("Marker name");
  await name.fill("Chorus");
  await name.press("Enter");
  await expect.poll(async () => (await markers(page))[0]?.name).toBe("Chorus");
  await marker.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Delete Marker" }).click();
  await expect.poll(async () => (await markers(page)).length).toBe(0);

  expect(errors).toEqual([]);
});
