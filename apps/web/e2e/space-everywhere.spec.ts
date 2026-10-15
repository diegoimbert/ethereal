// transport-keys, owner rule: "Space always does play / stop, no matter where we are in the
// app, unless we are inside a text box". Each control below is focused (a click, then a
// keyboard focus on it: the worst case), then Space: the transport toggles and the control
// is NOT pressed (browsers click a focused button on the Space keyup). Covered: a rack
// chain's mute (a chain row of the rack card: the owner's report), a track header's solo, the
// inspector mixer's mute and pan knob, a focused device card, a device window's knob, an open menu's item, a dialog's
// button; then a space typed in a text field (track rename) is typed, the transport unchanged.
//
// No sleeps: every step waits on UI or engine state (`window.__ether`, apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Device, Project, TransportState } from "@/generated";
import { addDevice, createTrack, deviceCard, launch, openDeviceTab, openDeviceWindow, playButton, headerButton } from "./ui";

interface Handle {
  state(): { project: Project | null; transport: TransportState | null };
}

const playing = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().transport?.playing ?? null);

const project = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

/**
 * Click `control` (unless `click: false`), focus it, press Space: the transport flips and
 * `state()` (the control's own state) stays what it was before the key.
 */
async function spacePlaysFrom(page: Page, control: Locator, state: () => Promise<unknown>, opts: { click?: boolean } = {}) {
  if (opts.click !== false) {
    const toggle = (await control.getAttribute("aria-pressed")) !== null;
    await control.click();
    // A toggle button's click lands in the engine a moment later: wait for it before the baseline.
    if (toggle) await expect(control).toHaveAttribute("aria-pressed", "true");
  }
  await control.focus();
  await expect(control).toBeFocused();
  const was = await playing(page);
  const before = await state();
  // Held (a second, auto-repeat keydown): still one toggle.
  await page.keyboard.down("Space");
  await page.keyboard.down("Space");
  await page.keyboard.up("Space");
  await expect.poll(() => playing(page)).toBe(!was);
  expect(await state()).toEqual(before);
}

test("Space plays / stops from any focused control, but types in text fields", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1600, height: 1200 });

  await launch(page, `Space everywhere ${Date.now()}`);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  const track = await createTrack(page, "Audio");
  await openDeviceTab(page, track.name);
  const inspector = page.getByTestId("inspector");

  // --- an Audio Effect Rack's chain mute / solo (the owner's report) ----------------------
  const before = new Set(Object.keys((await project(page))!.devices));
  await addDevice(page, "AudioEffectRack");
  let rack: Device | undefined;
  await expect
    .poll(async () => {
      rack = Object.values((await project(page))!.devices).find(
        (d) => !before.has(d.id) && d.kind.type === "Builtin" && d.kind.device.type === "AudioEffectRack",
      );
      return rack !== undefined;
    })
    .toBe(true);
  // A new effect rack comes with its first chain, a collapsible row on the rack's card
  // (ui/src/features/racks/RackPanel.tsx) with the track header's mute / solo.
  const card = deviceCard(page, rack!.id);
  const chains = card.getByRole("list", { name: "chains" }).first();
  await expect(chains.getByRole("listitem")).toHaveCount(1);
  const chainState = async () => Object.values((await project(page))!.rack_chains).map((c) => [c.mute, c.solo]);
  // A click mutes it (once); Space then plays and doesn't unmute it.
  const mute = chains.getByRole("button", { name: "mute chain 1", exact: true });
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  await spacePlaysFrom(page, mute, chainState, { click: false });
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  const solo = chains.getByRole("button", { name: "solo chain 1", exact: true });
  await solo.click();
  await expect(solo).toHaveAttribute("aria-pressed", "true");
  await spacePlaysFrom(page, solo, chainState, { click: false });
  // The chain row itself (Enter / Space would expand it): Space plays there, the row stays collapsed.
  const row = chains.getByRole("button", { name: "chain 1 chain", exact: true });
  await spacePlaysFrom(page, row, async () => [await chainState(), await row.getAttribute("aria-expanded")], { click: false });
  await expect(row).toHaveAttribute("aria-expanded", "false");

  // --- track header solo, inspector mixer mute and pan knob --------------------------------
  const headerSolo = await headerButton(page, track.name, `solo ${track.name}`);
  await spacePlaysFrom(page, headerSolo, () => headerSolo.getAttribute("aria-pressed"));
  const mixerMute = inspector.getByRole("button", { name: `mute ${track.name}` });
  await spacePlaysFrom(page, mixerMute, () => mixerMute.getAttribute("aria-pressed"));
  const pan = inspector.getByRole("slider", { name: `${track.name} pan` });
  await spacePlaysFrom(page, pan, () => pan.getAttribute("aria-valuenow"), { click: false });

  // --- a device window's knob ----------------------------------------------------------------
  const before2 = new Set(Object.keys((await project(page))!.devices));
  await addDevice(page, "Compressor");
  let comp: Device | undefined;
  await expect
    .poll(async () => {
      comp = Object.values((await project(page))!.devices).find((d) => !before2.has(d.id) && d.kind.type === "Builtin" && d.kind.device.type === "Compressor");
      return comp !== undefined;
    })
    .toBe(true);
  // The focused device card (#355: focusable, Enter opens its window): Space plays, no window.
  const compWin = page.locator(`[data-device-window="${comp!.id}"]`);
  await spacePlaysFrom(page, deviceCard(page, comp!.id), () => compWin.count(), { click: false });
  await expect(compWin).toHaveCount(0);
  const win = await openDeviceWindow(page, comp!.id);
  const knob = win.getByRole("slider").first();
  await spacePlaysFrom(page, knob, () => knob.getAttribute("aria-valuenow"), { click: false });

  // --- an open menu's item: plays, the menu stays open (Escape closes it) --------------------
  await page.getByRole("button", { name: "projects" }).click();
  const menu = page.getByRole("menu", { name: "project" });
  await expect(menu).toBeVisible();
  const item = menu.getByRole("menuitem").first();
  await spacePlaysFrom(page, item, async () => menu.isVisible(), { click: false });
  await expect(menu).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  // --- a dialog's button: plays, the dialog stays and nothing is pressed ------------------
  await page.getByRole("button", { name: "projects" }).click();
  await page.getByRole("menu", { name: "project" }).getByRole("menuitem", { name: "open project…", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "open project" });
  await expect(dialog).toBeVisible();
  const newBtn = dialog.getByRole("button", { name: "new project" });
  const name = await page.getByTestId("project-name").textContent();
  await spacePlaysFrom(page, newBtn, async () => [await dialog.isVisible(), await page.getByTestId("project-name").textContent()], { click: false });
  expect(await page.getByTestId("project-name").textContent()).toBe(name);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);

  // --- a text field keeps its spaces (rename the track) -------------------------------------
  const was = await playing(page);
  const field = inspector.getByRole("textbox", { name: "track name" });
  await field.click();
  await field.press("End");
  await page.keyboard.type(" two");
  await expect(field).toHaveValue(`${track.name} two`);
  expect(await playing(page)).toBe(was);
  await field.press("Enter");
  await expect.poll(async () => (await project(page))!.tracks[track.id]?.name).toBe(`${track.name} two`);
  expect(await playing(page)).toBe(was);

  expect(errors).toEqual([]);
});
