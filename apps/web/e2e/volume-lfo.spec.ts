// volume lfo (fx-modulation): the curve editor on the web build, against the real wasm
// engine. Inserts a Volume LFO, loads the "trance gate 1/16" factory preset from the
// header's preset menu (the editor draws its 22 points), adds a point with a double-click
// and undoes it.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, clickHeaderControl, createTrack, launch, openDeviceTab, openDeviceWindow, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

/** Points of the device's curve in the document. */
async function points(page: Page, id: string): Promise<number> {
  const k = (await project(page))!.devices[id]!.kind;
  return k.type === "Builtin" && k.device.type === "VolumeLfo" ? k.device.curve.points.length : -1;
}

test("volume lfo: a factory preset draws its curve, double-click adds a point", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  const track = await createTrack(page, "Audio");
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await project(page))!.devices));
  await addDevice(page, "VolumeLfo");
  let id = "";
  await expect
    .poll(async () => {
      const p = (await project(page))!;
      id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
      return id;
    })
    .not.toBe("");
  const win = await openDeviceWindow(page, id);
  const editor = win.getByTestId("widget-lfo-curve").getByRole("img");
  // A new one draws the default pump.
  await expect(editor).toHaveAttribute("data-points", "3");

  // Load "trance gate 1/16" from the header's preset menu.
  const card = page.locator(`section[data-device="${id}"]`);
  const trigger = card.getByRole("button", { name: "presets for volume lfo" });
  await clickHeaderControl(trigger);
  const panel = page.getByRole("dialog", { name: "presets for volume lfo" });
  await panel.getByRole("textbox", { name: "search presets" }).fill("trance");
  await panel.getByRole("button", { name: /trance gate 1\/16/ }).click();
  await expect.poll(() => points(page, id)).toBe(22);
  await expect(editor).toHaveAttribute("data-points", "22");
  await expect(editor.locator("[data-point]")).toHaveCount(22);
  await expect
    .poll(() =>
      editor
        .locator('[data-testid="lfo-curve-line"]')
        .getAttribute("points")
        .then((p) => p?.split(" ").length ?? 0),
    )
    .toBeGreaterThan(40);

  // Double-click in an empty spot (low in the first off step) adds a point: one undo step.
  const box = (await editor.boundingBox())!;
  await editor.dblclick({ position: { x: box.width * (1.2 / 16), y: box.height * 0.6 } });
  await expect.poll(() => points(page, id)).toBe(23);
  await expect(editor).toHaveAttribute("data-points", "23");
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => points(page, id)).toBe(22);
  expect(errors).toEqual([]);
});
