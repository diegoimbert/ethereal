// presets: the preset menu of a device header on the web build (wasm engine, user presets
// in the OPFS user library). Screenshots for the PR with `PRESETS_SHOTS=<dir>`:
//   PRESETS_SHOTS=/tmp/shots npx playwright test presets
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, launch, openDeviceTab, openOnLaunch, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function start(page: Page, theme?: string) {
  await page.setViewportSize({ width: 1440, height: 900 });
  if (theme) await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
}

/** A new MIDI track with a Synth; resolves with the Synth's device id. */
async function synth(page: Page): Promise<string> {
  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await project(page))!.devices));
  await addDevice(page, "Synth");
  let id = "";
  await expect
    .poll(async () => {
      const p = (await project(page))!;
      id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
      return id !== "";
    })
    .toBe(true);
  return id;
}

const param = async (page: Page, device: string, id: number) => (await project(page))!.devices[device]!.params[id];

test("load a factory preset, save / reload / delete a user preset", async ({ page }) => {
  await start(page);
  const id = await synth(page);
  const card = page.locator(`section[data-device="${id}"]`);
  const trigger = card.getByRole("button", { name: "Presets for Synth" });

  // Load "Sub Bass" (transpose -12): one undo step.
  await trigger.click();
  const panel = page.getByRole("dialog", { name: "Presets for Synth" });
  await panel.getByRole("textbox", { name: "Search presets" }).fill("sub");
  await panel.getByRole("button", { name: /Sub Bass/ }).click();
  await expect.poll(() => param(page, id, 1)).toBe(-12);
  await expect(trigger).toHaveText(/Sub Bass/);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => param(page, id, 1)).toBe(0);

  // Save a user preset.
  await trigger.click();
  await panel.getByRole("button", { name: "Save preset…" }).click();
  const save = page.getByRole("dialog", { name: "Save preset for Synth" });
  await save.getByRole("textbox", { name: "Preset name" }).fill("E2E Lead");
  await save.getByRole("textbox", { name: "Preset tags" }).fill("lead");
  await save.getByRole("button", { name: "Save" }).click();
  await expect(save).toBeHidden();

  // It survives a reload (OPFS user library) and loads onto a fresh synth.
  await page.reload();
  await openOnLaunch(page);
  const id2 = await synth(page);
  const card2 = page.locator(`section[data-device="${id2}"]`);
  await card2.getByRole("button", { name: "Presets for Synth" }).click();
  const user = panel.getByRole("region", { name: "User presets" });
  await expect(user.getByRole("button", { name: /^E2E Lead/ })).toBeVisible();

  // Delete it from its menu.
  await user.getByRole("button", { name: "More actions for E2E Lead" }).click();
  await page.getByRole("menuitem", { name: "Delete Preset…" }).click();
  const del = page.getByRole("dialog", { name: "Delete “E2E Lead”?" });
  await del.getByRole("button", { name: "Delete" }).click();
  await expect(del).toBeHidden();
  await card2.getByRole("button", { name: "Presets for Synth" }).click();
  await expect(panel.getByRole("region", { name: "User presets" })).toHaveCount(0);
});

const shots = process.env.PRESETS_SHOTS;
for (const theme of ["dark", "light"]) {
  test(`screenshots: preset menu + save dialog (${theme})`, async ({ page }) => {
    test.skip(!shots, "set PRESETS_SHOTS=<dir> to capture screenshots");
    await start(page, theme);
    const id = await synth(page);
    const card = page.locator(`section[data-device="${id}"]`);
    const trigger = card.getByRole("button", { name: "Presets for Synth" });
    // A user preset so both groups show.
    await trigger.click();
    await page.getByRole("button", { name: "Save preset…" }).click();
    const save = page.getByRole("dialog", { name: "Save preset for Synth" });
    await save.getByRole("textbox", { name: "Preset name" }).fill("Night Drive");
    await save.getByRole("textbox", { name: "Preset tags" }).fill("lead, dark");
    await save.getByRole("button", { name: "Save" }).click();
    await expect(save).toBeHidden();

    await trigger.click();
    const panel = page.getByRole("dialog", { name: "Presets for Synth" });
    await expect(panel.getByRole("region", { name: "User presets" })).toBeVisible();
    await panel.getByRole("button", { name: /Soft Pad/ }).hover();
    await page.waitForTimeout(300);
    const a = (await card.boundingBox())!;
    const b = (await panel.boundingBox())!;
    const x = Math.min(a.x, b.x) - 8;
    const y = Math.min(a.y, b.y) - 8;
    await page.screenshot({
      path: `${shots}/presets-menu-${theme}.png`,
      clip: { x, y, width: Math.max(a.x + a.width, b.x + b.width) - x + 8, height: Math.max(a.y + a.height, b.y + b.height) - y + 8 },
    });

    await panel.getByRole("button", { name: "Save preset…" }).click();
    await save.getByRole("textbox", { name: "Preset name" }).fill("Warm Bass");
    await save.getByRole("textbox", { name: "Preset tags" }).fill("bass, warm");
    await expect(save.getByRole("button", { name: "Save" })).toBeVisible();
    await page.waitForTimeout(300);
    await save.screenshot({ path: `${shots}/presets-save-${theme}.png` });
  });
}
