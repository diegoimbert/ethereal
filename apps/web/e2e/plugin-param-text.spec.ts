// #424: a plugin's knobs show the plugin's own display text (`Plugin::ParamTexts`; on the
// desktop: CLAP `value_to_text`, VST3
// `getParamStringByValue` + units). Runs the web build on the mock transport (`?mock`), whose
// "Mock Mega" plugin reads its Cutoff params as Hz; params without a text keep the UI's own
// formatting. `PLUGIN_TEXT_SHOTS=<dir>` also writes the PR screenshot.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openDeviceTab, playButton, showDeviceParams } from "./ui";

const MEGA = "dev.ethereal.mock.mega";
const shots = process.env.PLUGIN_TEXT_SHOTS;

interface Handle {
  state(): { project: Project | null };
}
interface MockHandle {
  send(command: unknown): Promise<unknown>;
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
const send = (page: Page, command: unknown) => page.evaluate((c) => (window as unknown as { __etherMock: MockHandle }).__etherMock.send(c), command);

test("plugin knobs show the plugin's own value text", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/?mock");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const p = (await project(page))!;
  const track = Object.values(p.tracks).find((t) => t.kind === "Audio") ?? Object.values(p.tracks).find((t) => t.kind !== "Master")!;
  await openDeviceTab(page, track.name);
  const id = `e2e-text-${Date.now()}`;
  await send(page, {
    domain: "Device",
    command: { type: "Insert", id, track: track.id, device: { type: "Plugin", plugin_id: MEGA, sandboxed: null }, before: null },
  });
  const card = page.locator(`section[data-device="${id}"]`);
  await expect(card).toBeVisible();
  await card.scrollIntoViewIfNeeded();

  const list = (await showDeviceParams(page, id)).getByRole("list", { name: "Mock Mega parameters" });
  const value = (param: number) => list.locator(`[data-param-row="${param}"] .eth-param__value`);
  // "Cutoff 1" (plain 0..1): the plugin's text; "Gain 0" (dB): the UI's formatting.
  await expect(value(1)).toHaveText("20 Hz");
  await expect(value(0)).toHaveText("0.0 dB");

  await send(page, { domain: "Device", command: { type: "SetParam", device: id, param: 1, value: 0.5 } });
  await expect(value(1)).toHaveText("632 Hz");
  await send(page, { domain: "Device", command: { type: "SetParam", device: id, param: 9, value: 0.9 } });
  await expect(value(9)).toHaveText("10.02 kHz");

  await page.mouse.move(0, 0);
  if (shots) await card.screenshot({ path: `${shots}/plugin-param-text.png` });
  expect(errors).toEqual([]);
});
