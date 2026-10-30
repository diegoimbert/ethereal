// Factory devices open in their own window, like a plugin's editor; the device
// header has two independent toggles: the window ("open window" icon, left) and the inline
// parameter list ("Parameters", right; capped height). On the real web engine: an EQ's
// window drives the engine, the inline list too, and neither toggle touches the other's
// state. On the mock engine (no plugin hosting on the web): a plugin's header has the same
// toggles. `DEVICE_WINDOW_SHOTS=<dir>` also writes the PR screenshots (dark and light).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import {
  addDevice,
  closeDeviceWindow,
  createTrack,
  deviceCard,
  launch,
  moveDeviceWindow,
  newProject,
  selectTrack,
  openDeviceTab,
  openDeviceWindow,
  playButton,
  showDeviceParams,
} from "./ui";

interface Handle {
  state(): { project: Project | null };
}
interface MockHandle {
  send(command: unknown): Promise<unknown>;
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
const param = async (page: Page, device: string, id: number, fallback: number) => (await project(page))!.devices[device]?.params[id] ?? fallback;

const shots = process.env.DEVICE_WINDOW_SHOTS;
// Crisp PR screenshots.
if (shots) test.use({ deviceScaleFactor: 2 });
/** `ether_devices::eq`: the output gain (dB). */
const OUTPUT = 40;
/** `--eth-size-device-params-max-height`. */
const PARAMS_MAX_PX = 240;

async function insertEq(page: Page, theme: string, keepWindow = false): Promise<string> {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const track = await createTrack(page, "Audio");
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await project(page))!.devices));
  await addDevice(page, "Eq", { keepWindow });
  let id = "";
  await expect
    .poll(async () => {
      const p = (await project(page))!;
      id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
      return id !== "";
    })
    .toBe(true);
  await expect(deviceCard(page, id)).toBeVisible();
  return id;
}

test("a factory device: its window and its inline parameters, independent", async ({ page }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const id = await insertEq(page, "dark");
  const card = deviceCard(page, id);
  const header = card.locator(":scope > header");
  const windowToggle = header.getByRole("button", { name: "open Parametric EQ window" });
  const paramsToggle = header.getByRole("button", { name: "show Parametric EQ parameters" });
  const win = page.locator(`[data-device-window="${id}"]`);

  // The card is its header: no chevron, both toggles off, the EQ's UI nowhere inline.
  await expect(header.getByRole("button", { name: /^(Expand|Collapse) / })).toHaveCount(0);
  await expect(windowToggle).toHaveAttribute("aria-pressed", "false");
  await expect(paramsToggle).toHaveAttribute("aria-pressed", "false");
  await expect(paramsToggle).toHaveAttribute("title", "show parameters");
  await expect(card.getByTestId("widget-eq-curve")).toHaveCount(0);
  await expect(card.locator("[data-param]")).toHaveCount(0);
  await expect(win).toHaveCount(0);

  // The window: the EQ's own UI, above the arrangement; the parameters stay hidden.
  await windowToggle.click();
  await expect(windowToggle).toHaveAttribute("aria-pressed", "true");
  await expect(win).toBeVisible();
  await expect(win.getByTestId("widget-eq-curve")).toBeVisible();
  await expect(card.getByTestId("device-params")).toHaveCount(0);
  await expect(paramsToggle).toHaveAttribute("aria-pressed", "false");
  if (shots) {
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${shots}/device-window-dark.png` });
    await header.screenshot({ path: `${shots}/header-window-open-dark.png` });
  }

  // A parameter changed in the window reaches the engine (one undo step).
  const output = win.getByRole("slider", { name: "output" });
  await output.focus();
  await output.press("PageUp");
  await expect.poll(() => param(page, id, OUTPUT, 0)).toBeGreaterThan(0);
  const raised = await param(page, id, OUTPUT, 0);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => param(page, id, OUTPUT, 0)).toBe(0);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(() => param(page, id, OUTPUT, 0)).toBe(raised);

  // The window moves by its title bar and comes back where it was.
  await moveDeviceWindow(page, id, { x: 120, y: 160 });
  await closeDeviceWindow(page, id);
  await expect(windowToggle).toHaveAttribute("aria-pressed", "false");
  await windowToggle.click();
  await expect.poll(async () => Math.abs((await win.boundingBox())!.x - 120)).toBeLessThanOrEqual(1.5);

  // The parameters, inline: the window stays open; capped, scrolling inside.
  const params = await showDeviceParams(page, id);
  await expect(paramsToggle).toHaveAttribute("aria-expanded", "true");
  await expect(paramsToggle).toHaveAttribute("title", "hide parameters");
  await expect(windowToggle).toHaveAttribute("aria-pressed", "true");
  expect((await params.boundingBox())!.height).toBeLessThanOrEqual(PARAMS_MAX_PX + 1);
  const list = params.getByRole("list", { name: "Parametric EQ parameters" });
  expect(await list.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  // Output from the inline list: the engine follows, and the window shows the same value.
  const row = params.locator(`[data-param-row="${OUTPUT}"]`);
  await params.getByRole("searchbox", { name: "search Parametric EQ parameters" }).fill("output");
  await expect(row).toBeVisible();
  const inline = row.getByRole("slider");
  await inline.focus();
  await inline.press("PageDown");
  await expect.poll(() => param(page, id, OUTPUT, 0)).toBeLessThan(raised);
  await expect(output).toHaveAttribute("aria-valuetext", (await inline.getAttribute("aria-valuetext"))!);
  if (shots) {
    await page.mouse.move(0, 0);
    await card.screenshot({ path: `${shots}/params-expanded-dark.png` });
  }

  // Closing the window keeps the parameters; hiding them keeps the window.
  await closeDeviceWindow(page, id);
  await expect(card.getByTestId("device-params")).toBeVisible();
  await windowToggle.click();
  await expect(win).toBeVisible();
  await header.hover();
  await paramsToggle.click();
  await expect(card.getByTestId("device-params")).toHaveCount(0);
  await expect(win).toBeVisible();
  if (shots) {
    await page.mouse.move(0, 0);
    await header.screenshot({ path: `${shots}/header-closed-dark.png` });
  }

  // Deleting the device closes its window.
  await header.getByRole("button", { name: "remove Parametric EQ" }).click();
  await expect(win).toHaveCount(0);
  expect(errors).toEqual([]);
});

// A device added from the library opens its window (as a plugin opens its editor).
test("a factory device added from the library opens its window", async ({ page }) => {
  test.setTimeout(60_000);
  const id = await insertEq(page, "dark", true);
  const win = page.locator(`[data-device-window="${id}"]`);
  await expect(win).toBeVisible();
  await expect(win.getByTestId("widget-eq-curve")).toBeVisible();
  // Below the top bar, the "Open on desktop" band above it included: the transport stays clickable.
  await win.evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));
  const bar = (await page.getByRole("toolbar", { name: "transport" }).boundingBox())!;
  expect((await win.boundingBox())!.y).toBeGreaterThanOrEqual(bar.y + bar.height);
  const toggle = deviceCard(page, id).locator(":scope > header").getByRole("button", { name: "open Parametric EQ window" });
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  // It moves and closes like any device window.
  await moveDeviceWindow(page, id, { x: 200, y: 140 });
  await closeDeviceWindow(page, id);
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
});

// A device window opens only from its button (0560b776); a click on the card selects it, for
// copy / paste (fa9f99e7, deviceClipboard.ts). The card's controls keep doing their own thing.
test("a click on a factory device's card selects it; its window opens from its button only", async ({ page }) => {
  test.setTimeout(60_000);
  const id = await insertEq(page, "dark");
  const card = deviceCard(page, id);
  const header = card.locator(":scope > header");
  const toggle = header.getByRole("button", { name: "open Parametric EQ window" });
  const win = page.locator(`[data-device-window="${id}"]`);
  // The card's controls: the power button bypasses, no window.
  await header.getByRole("button", { name: "bypass Parametric EQ" }).click();
  await expect(win).toHaveCount(0);
  await header.getByRole("button", { name: "enable Parametric EQ" }).click();
  // The name: selects the card (and focuses it, for ⌘C), no window, also once a
  // double-click's wait is over.
  await card.locator(".eth-device__name").click();
  await expect(card).toHaveClass(/eth-device--selected/);
  await expect(card).toBeFocused();
  await page.waitForTimeout(400);
  await expect(win).toHaveCount(0);
  // The header's empty space and Enter on the focused card: no window either.
  await header.click({ position: { x: (await header.boundingBox())!.width / 2, y: 4 } });
  await card.focus();
  await page.keyboard.press("Enter");
  await page.waitForTimeout(400);
  await expect(win).toHaveCount(0);
  await expect(card).toHaveClass(/eth-device--selected/);
  // The toggle opens it, and closes it.
  await toggle.click();
  await expect(win).toBeVisible();
  await expect(win.getByTestId("widget-eq-curve")).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  if (shots) {
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${shots}/card-selected-window-open.png` });
  }
  await toggle.click();
  await expect(win).toHaveCount(0);
  // A double-click on the name renames, no window.
  await card.locator(".eth-device__name").dblclick();
  await expect(card.getByRole("textbox", { name: "rename Parametric EQ" })).toBeVisible();
  await page.waitForTimeout(400);
  await expect(win).toHaveCount(0);
  await page.keyboard.press("Escape");
});

// ⌘V with no device selected (a click on the chain's empty space) appends at the chain's end;
// another ⌘V appends again (deviceClipboard.ts).
test("a device pasted with none selected goes at the end of the chain", async ({ page }) => {
  test.setTimeout(60_000);
  const id = await insertEq(page, "dark");
  const card = deviceCard(page, id);
  const track = (await project(page))!.devices[id]!.track;
  const chain = (): Promise<string[]> =>
    project(page).then((p) =>
      Object.values(p!.devices)
        .filter((d) => d.track === track && d.chain == null && d.pad === null)
        .sort((a, b) => (a.order < b.order ? -1 : 1))
        .map((d) => d.id),
    );
  await card.locator(".eth-device__name").click();
  await page.keyboard.press("ControlOrMeta+c");
  // The chain's empty space: no device selected, the chain focused.
  const space = page.locator("[data-paste-chain]").filter({ has: card });
  await space.locator(":scope > .eth-devices__end").click();
  await expect(card).not.toHaveClass(/eth-device--selected/);
  await expect(space).toBeFocused();
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(() => chain().then((c) => c.length)).toBe(2);
  const [first, pasted] = await chain();
  expect(first).toBe(id);
  await expect(deviceCard(page, pasted!)).toHaveClass(/eth-device--selected/);
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(() => chain().then((c) => c.length)).toBe(3);
  expect((await chain()).slice(0, 2)).toEqual([id, pasted]);
});

test("a plugin's header (mock engine): the same two toggles, independent", async ({ page }) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/?mock");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const p = (await project(page))!;
  const track = Object.values(p.tracks).find((t) => t.kind === "Audio") ?? Object.values(p.tracks).find((t) => t.kind !== "Master")!;
  await openDeviceTab(page, track.name);
  const id = `e2e-mega-${Date.now()}`;
  await page.evaluate(
    ([id, track]) =>
      (window as unknown as { __etherMock: MockHandle }).__etherMock.send({
        domain: "Device",
        command: { type: "Insert", id, track, device: { type: "Plugin", plugin_id: "dev.ethereal.mock.mega", sandboxed: null }, before: null },
      }),
    [id, track.id] as const,
  );
  const card = deviceCard(page, id);
  const header = card.locator(":scope > header");
  const windowToggle = header.getByRole("button", { name: "open Mock Mega window" });
  // No plugin hosting on the web: the plugin's editor can't open here. The toggle says why
  // (aria-disabled, so its tooltip still shows) instead of doing nothing.
  await expect(windowToggle).toBeDisabled();
  await expect(windowToggle).toHaveAttribute("aria-disabled", "true");
  await expect(windowToggle).toHaveAttribute("title", /plugins don't run in the browser/);
  await expect(windowToggle).toHaveAttribute("aria-pressed", "false");
  // The parameters: the generic list, inline (and a factory window never opens for a plugin).
  await showDeviceParams(page, id);
  await expect(card.getByRole("list", { name: "Mock Mega parameters" })).toBeVisible();
  await expect(windowToggle).toHaveAttribute("aria-pressed", "false");
  // aria-disabled: a click reaches it and opens nothing (no window, no error).
  await windowToggle.click({ force: true });
  await expect(page.locator(`[data-device-window="${id}"]`)).toHaveCount(0);
  await expect(page.locator("[data-remote-plugin-window]")).toHaveCount(0);
  await expect(card.getByTestId("device-params")).toBeVisible();
  // A click on the card asks for the plugin's editor, like the toggle: never a factory window.
  await header.click({ position: { x: (await header.boundingBox())!.width / 2, y: 4 } });
  await expect(page.locator(`[data-device-window="${id}"]`)).toHaveCount(0);
  await expect(card.getByTestId("device-params")).toBeVisible();
});

if (shots) {
  test("light theme screenshots", async ({ page }) => {
    test.setTimeout(90_000);
    const id = await insertEq(page, "light");
    const card = deviceCard(page, id);
    const header = card.locator(":scope > header");
    await page.mouse.move(0, 0);
    await header.screenshot({ path: `${shots}/header-closed-light.png` });
    await openDeviceWindow(page, id);
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${shots}/device-window-light.png` });
    await header.screenshot({ path: `${shots}/header-window-open-light.png` });
    await showDeviceParams(page, id);
    await page.mouse.move(0, 0);
    await card.screenshot({ path: `${shots}/params-expanded-light.png` });
  });
}

test("a drum rack's window is its pad editor, not just volume and pan", async ({ page }) => {
  test.setTimeout(60_000);
  await launch(page);
  await newProject(page, `Drum window ${Date.now()}`);
  const track = await createTrack(page, "Midi");
  await selectTrack(page, track.name);
  await addDevice(page, "DrumRack", { keepWindow: true });
  const win = page.locator("[data-device-window]").last();
  await expect(win).toBeVisible();
  await expect(win.getByRole("grid", { name: "drum pads" })).toBeVisible();
  await expect(win.getByRole("combobox", { name: "pad bank" }).or(win.getByLabel("pad bank"))).toBeVisible();
});
