// device-ui: screenshots of built-in device panels (the shared renderer), for PR review
// and the owner's UX pass. Skipped unless `DEVICE_UI_SHOTS=<dir>` is set:
//   DEVICE_UI_SHOTS=/tmp/shots DEVICE_UI_THEME=dark npx playwright test device-ui-shots
// One bounded test per device (60 s each), so a stuck capture fails alone instead of hanging
// the run. `DEVICE_UI_ONLY=Synth,Delay` narrows the list.
import { expect, test, type Page } from "@playwright/test";
import type { BuiltinDeviceType, Project } from "@/generated";
import { addDevice, createTrack, openDeviceTab, playButton } from "./ui";

const dir = process.env.DEVICE_UI_SHOTS;
const theme = process.env.DEVICE_UI_THEME ?? "dark";

/** The v0.1 built-ins (all have declared layouts); v0.2 devices bring their own shots. */
const V01: BuiltinDeviceType[] = ["Synth", "Sampler", "DrumRack", "Compressor", "Delay", "Eq", "Reverb", "Limiter", "Utility"];
const types = process.env.DEVICE_UI_ONLY?.split(",").filter(Boolean) ?? V01;

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

test.skip(!dir, "set DEVICE_UI_SHOTS=<dir> to capture device screenshots");

for (const type of types) {
  test(`device panel: ${type} (${theme})`, async ({ page }) => {
    test.setTimeout(60_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    await page.goto("/");
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);

    const track = await createTrack(page, type === "Synth" || type === "Sampler" || type === "DrumRack" ? "Midi" : "Audio");
    await openDeviceTab(page, track.name);
    const before = new Set(Object.keys((await project(page))!.devices));
    await addDevice(page, type);
    let id = "";
    await expect
      .poll(async () => {
        const p = (await project(page))!;
        id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
        return id !== "";
      })
      .toBe(true);
    const card = page.locator(`section[data-device="${id}"]`);
    await expect(card).toBeVisible();
    // Wait for the descriptor (the "Loading…" status goes away).
    await expect(card.getByText("Loading…")).toHaveCount(0);
    await card.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(250);
    await card.screenshot({ path: `${dir}/${type}-${theme}.png`, timeout: 15_000 });
  });
}
