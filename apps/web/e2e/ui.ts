// Shared e2e gestures for the app's UI patterns. Import from here instead of re-deriving the
// gesture in each spec, so a UX change (a new flow, a kit control reworked) is fixed in one
// place. Clip gestures live in ./clips.ts.
import { expect, type Locator, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";

interface Handle {
  state(): { project: Project | null };
}

const tracksOf = (page: Page): Promise<Track[]> =>
  page.evaluate(() => Object.values((window as unknown as { __ether: Handle }).__ether.state().project?.tracks ?? {}));

/**
 * Adds a track the way a user does: "Add track" (the arrangement's corner) opens a menu of
 * the kinds ("Create MIDI track" / "Create audio track"); a click adds it at the end. Resolves
 * with the new track once the controller has it.
 */
export async function createTrack(page: Page, kind: "Midi" | "Audio"): Promise<Track> {
  const before = new Set((await tracksOf(page)).map((t) => t.id));
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: kind === "Midi" ? "Create MIDI track" : "Create audio track" }).click();
  let created: Track | undefined;
  await expect
    .poll(async () => {
      created = (await tracksOf(page)).find((t) => !before.has(t.id) && t.kind === kind);
      return created !== undefined;
    })
    .toBe(true);
  if (kind === "Midi") {
    // A new MIDI track opens its instrument picker (Synth stays unless one is picked): dismiss
    // it so it doesn't cover the rest of the page.
    const picker = page.getByRole("dialog", { name: `choose an instrument for ${created!.name}` });
    await expect(picker).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(picker).toBeHidden();
  }
  return created!;
}

/**
 * Chooses an option of a kit `Select` like a user: opens the list (a `combobox` button) and
 * clicks the option. `select` is the combobox, or its accessible name (exact) within `scope`.
 * `option` is the option's label (exact) or `{ value }` for its value.
 */
export async function pickOption(scope: Page | Locator, select: string | Locator, option: string | { value: string }): Promise<void> {
  const trigger = typeof select === "string" ? scope.getByRole("combobox", { name: select, exact: true }) : select;
  const page = "page" in scope ? scope.page() : scope;
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  // The list this trigger controls (portaled to <body>; another may still be animating out).
  const listbox = page.locator(`[id="${await trigger.getAttribute("aria-controls")}"]`);
  const item =
    typeof option === "string" ? listbox.getByRole("option", { name: option, exact: true }) : listbox.locator(`[role="option"][data-value="${option.value}"]`);
  await item.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

/**
 * A built-in device's text (a device, parameter or choice name, as the engine names it:
 * "Instrument Rack", "Attack Time") as the UI shows it, whatever its case (the UI shows it
 * through the catalog: "instrument rack").
 */
export const ci = (name: string): RegExp => new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`, "i");

/**
 * Adds a device from a rack chain's / pad's "+ Add device…" picker: the shared device
 * library popover (search bar, rows named by device, plugin settings footer).
 */
export async function pickDevice(scope: Page | Locator, trigger: string, row: string): Promise<void> {
  const page = "page" in scope ? scope.page() : scope;
  await scope.getByRole("button", { name: trigger, exact: true }).click();
  const picker = page.getByRole("dialog", { name: trigger, exact: true });
  await expect(picker.getByRole("searchbox")).toBeVisible();
  await picker.getByRole("button", { name: ci(row) }).click();
  await expect(picker).toHaveCount(0);
}

/**
 * Selects a track the way a user does: a click on its header card, at its left edge. The
 * card's center lands on its mute toggle, and the name can't be clicked: at the default
 * header width it is squeezed to 0 px (an app layout bug).
 */
export async function selectTrack(page: Page, trackName: string): Promise<void> {
  const header = page.getByRole("group", { name: `${trackName} track` });
  const box = (await header.boundingBox())!;
  await header.click({ position: { x: 6, y: box.height / 2 } });
}

/**
 * Shows a track's devices: selecting the track opens the inspector with its device chain.
 * Resolves once the chain's sections are there.
 */
export async function openDeviceTab(page: Page, trackName: string): Promise<void> {
  await selectTrack(page, trackName);
  await expect(page.getByTestId("inspector").getByRole("group", { name: "audio effects" })).toBeVisible();
}

/** A device's card (its header, and its inline parameter list or rack chains when shown). */
export function deviceCard(page: Page, id: string): Locator {
  return page.locator(`section[data-device="${id}"]`);
}

/**
 * Clicks one of a device header's hover-only controls (the presets menu, "+" modulator) like
 * a user: they take no room until the header is hovered, so hover it first.
 */
export async function clickHeaderControl(control: Locator): Promise<void> {
  await control.locator("xpath=ancestor::header[1]").hover({ position: { x: 4, y: 4 } });
  await control.click();
}

/**
 * Opens a factory device's window from its card's header toggle (the device's own
 * UI, the layout, is there and never inline) and returns it, once its layout is loaded.
 */
export async function openDeviceWindow(page: Page, id: string): Promise<Locator> {
  const toggle = deviceCard(page, id).locator(":scope > header .eth-device__window-toggle");
  const win = page.locator(`[data-device-window="${id}"]`);
  // Another device's window can float over the chain (effects open wider than the base,
  // window/sizes.ts): the toggle is pressed directly then, as it is not what is under test.
  if ((await toggle.getAttribute("aria-pressed")) !== "true") await toggle.dispatchEvent("click");
  // Already open: a press in it brings it on top of the other windows.
  else await win.dispatchEvent("pointerdown");
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect(win).toBeVisible();
  await expect(win.getByText("loading…")).toHaveCount(0);
  // Done flying in (its box is final).
  await win.evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));
  return win;
}

/** Drags a device window by its title bar so its top-left corner lands at `to` (page px). */
export async function moveDeviceWindow(page: Page, id: string, to: { x: number; y: number }): Promise<void> {
  const win = page.locator(`[data-device-window="${id}"]`);
  const bar = win.locator(":scope > header .eth-fwin__title");
  await win.evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));
  const box = (await win.boundingBox())!;
  const grab = (await bar.boundingBox())!;
  const gx = grab.x + 4;
  const gy = grab.y + grab.height / 2;
  await page.mouse.move(gx, gy);
  await page.mouse.down();
  await page.mouse.move(to.x + (gx - box.x), to.y + (gy - box.y), { steps: 8 });
  await page.mouse.up();
  // Within a pixel (sub-pixel pointer positions).
  await expect.poll(async () => Math.abs((await win.boundingBox())!.x - to.x)).toBeLessThanOrEqual(1.5);
}

/** Closes a factory device's window (its title bar's close button). */
export async function closeDeviceWindow(page: Page, id: string): Promise<void> {
  const win = page.locator(`[data-device-window="${id}"]`);
  await win
    .locator(":scope > header")
    .getByRole("button", { name: /^close .* window$/ })
    .click();
  await expect(win).toHaveCount(0);
}

// ---- Rack chains (ui/src/features/racks/RackPanel.tsx) ---------------------------------------
// A rack's card lists its chains as collapsible rows (b8ad9f79): a row has the track header's
// mute / solo, and expanded, the chain's devices in sections with their "+ Add …" cards. An
// instrument rack's "+ Add instrument" makes a chain with the picked instrument; an effect
// rack's "+ Add chain" an empty, expanded chain. A new effect rack comes with a first chain
// (4d8c128d). A rack on a chain draws its own panel inside that chain's row, so these
// helpers stay on the panel's own chains (`:scope >`), never a nested rack's.

/** The chains panel of a rack (on its card). */
export const rackPanel = (page: Page, rackId: string): Locator => page.locator(`.eth-rack[data-rack="${rackId}"]`);

const chainList = (panel: Locator): Locator => panel.locator(':scope > [role="list"][aria-label="chains"]');

/** A rack panel's own chain rows, in order (not a nested rack's). */
export const rackChains = (panel: Locator): Locator => chainList(panel).locator(':scope > [role="listitem"]');

/** A rack panel's own chain by its name (the engine's, e.g. "chain 1"). */
export const rackChain = (panel: Locator, name: string): Locator => chainList(panel).locator(`:scope > [role="listitem"][aria-label="${name}"]`);

/**
 * Adds a chain like a user: an effect rack's "+ Add chain" (the chain opens, empty), or an
 * instrument rack's "+ Add instrument" and a pick of `instrument` in its library (a collapsed
 * chain with it, whose window opens: closed again unless `keepWindow`, as `addDevice` does).
 * Resolves once the row is there.
 */
export async function addRackChain(panel: Locator, instrument?: string, opts: { keepWindow?: boolean } = {}): Promise<void> {
  const page = panel.page();
  const count = await rackChains(panel).count();
  const windowsBefore = await windowIds(page);
  // The add card closes the list, outside the chain rows (a nested rack's is inside one).
  await chainList(panel).locator(':scope > .eth-rack__add-chain, :scope > :not([role="listitem"]) .eth-rack__add-chain').click();
  if (instrument !== undefined) {
    const picker = page.getByRole("dialog", { name: "add instrument", exact: true });
    await expect(picker.getByRole("searchbox")).toBeVisible();
    await picker.getByRole("button", { name: ci(instrument) }).click();
    await expect(picker).toHaveCount(0);
    await afterAdd(page, windowsBefore, instrument, opts.keepWindow);
  }
  await expect(rackChains(panel)).toHaveCount(count + 1);
}

/** Expands a rack panel's chain row (its devices show); resolves with the chain once open. */
export async function expandRackChain(panel: Locator, name: string): Promise<Locator> {
  const chain = rackChain(panel, name);
  const row = chain.locator(":scope > .eth-rack__row");
  // By its chevron: the row's middle has its controls (mute, solo, fader...), which don't toggle it.
  if ((await row.getAttribute("aria-expanded")) !== "true") await row.locator(":scope > .eth-rack__chevron").click();
  await expect(row).toHaveAttribute("aria-expanded", "true");
  // Done unfolding (the add cards are where they will stay).
  await expect(chain.locator(":scope > .eth-rack__fold--open")).toHaveCount(1);
  return chain;
}

/**
 * Adds a device to a rack panel's chain from the expanded chain's section card (`trigger`:
 * "add audio effect", "add MIDI effect" or "add instrument"), picking `row` in the library.
 */
export async function addToRackChain(panel: Locator, chainName: string, trigger: string, row: string): Promise<void> {
  const chain = await expandRackChain(panel, chainName);
  await pickDevice(chain.locator(":scope > .eth-rack__fold .eth-rack__body").first(), trigger, row);
}

/** Shows a device card's inline parameter list (the header's "Parameters" toggle). */
export async function showDeviceParams(page: Page, id: string): Promise<Locator> {
  const card = deviceCard(page, id);
  const toggle = card.locator(":scope > header .eth-device__params-toggle");
  // Shown while the header is hovered (or the list is open).
  await card.locator(":scope > header").hover();
  if ((await toggle.getAttribute("aria-pressed")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  const params = card.getByTestId("device-params").first();
  await expect(params).toBeVisible();
  return params;
}

/**
 * Inserts a built-in device (by its type, e.g. "Compressor") from the inspector chain's
 * section pickers (each section's "+" opens the device library of its kind; a row's
 * `data-key` is `builtin:<Type>`). An instrument on a track that already has one replaces it:
 * the instrument slot only offers "+" while empty, so the current one is removed first.
 */
export async function addDevice(page: Page, type: string, opts: { keepWindow?: boolean } = {}): Promise<void> {
  const inspector = page.getByTestId("inspector");
  const before = await windowIds(page);
  const row = page.locator(`.eth-devlib__row[data-key="builtin:${type}"]`);
  /** Open `section`'s picker; true when `type` is listed there (else the picker is closed again). */
  const tryPicker = async (section: string) => {
    const add = inspector.getByRole("group", { name: section }).locator(".eth-devices__section-add");
    if (!(await add.count())) return false;
    await add.first().click();
    await expect(page.locator(".eth-devlib__row").first()).toBeVisible();
    if (await row.count()) return true;
    await page.keyboard.press("Escape");
    return false;
  };
  for (const section of ["audio effects", "MIDI effects", "instrument"]) {
    if (await tryPicker(section)) {
      await row.click();
      return afterAdd(page, before, type, opts.keepWindow);
    }
  }
  // An instrument for a filled slot: empty it, then pick it.
  const slot = inspector.getByRole("group", { name: "instrument" });
  await slot
    .getByRole("button", { name: /^remove / })
    .first()
    .click();
  await expect(slot.locator(".eth-devices__section-add")).toHaveCount(1);
  if (!(await tryPicker("instrument"))) throw new Error(`no device ${type} in the pickers`);
  await row.click();
  await afterAdd(page, before, type, opts.keepWindow);
}

/**
 * Built-in devices without a window (`hasWindow`, ui/src/features/devices/window): the
 * instrument / audio effect / MIDI effect racks, whose macros and chains are on their card.
 */
const NO_WINDOW = new Set(["InstrumentRack", "AudioEffectRack", "MidiEffectRack"]);

/**
 * After an add: wait for the picker to close and the new device's window to open (a
 * device added from the library opens its window); closed again unless `keepWindow`. A chain
 * rack has no window: only the picker closing is waited for (callers wait for the device).
 */
async function afterAdd(page: Page, windowsBefore: string[], type: string, keepWindow = false): Promise<void> {
  await expect(page.locator(".eth-devlib")).toHaveCount(0);
  if (NO_WINDOW.has(type)) return;
  // A new window (by device id): adding an instrument can replace one whose window was open,
  // so the count alone doesn't say whether the new device's window opened.
  let added = "";
  await expect
    .poll(async () => {
      const ids = await windowIds(page);
      added = ids.find((id) => !windowsBefore.includes(id)) ?? "";
      return added;
    })
    .not.toBe("");
  if (!keepWindow) {
    const win = page.locator(`[data-device-window="${added}"]`);
    await win
      .locator(":scope > header")
      .getByRole("button", { name: /^close .* window$/ })
      .click();
    await expect(win).toHaveCount(0);
  }
  await page.waitForTimeout(100);
}

/**
 * Puts a Drum Rack on a MIDI track like a user: the inspector's instrument picker (it replaces
 * the track's instrument) opens its window, the pad editor. The drawer's "Drum Rack" tab is
 * gone (e6d68591). Resolves with the rack's id and its window.
 */
export async function addDrumRack(page: Page, trackName: string): Promise<{ id: string; win: Locator }> {
  await openDeviceTab(page, trackName);
  const before = await windowIds(page);
  await addDevice(page, "DrumRack", { keepWindow: true });
  const id = (await windowIds(page)).find((w) => !before.includes(w))!;
  const win = page.locator(`[data-device-window="${id}"]`);
  await expect(win.getByRole("grid", { name: "drum pads" })).toBeVisible();
  return { id, win };
}

/**
 * A drum pad's own button in the pad grid (`Mute <pad>`, `Solo <pad>`, `Preview <pad>`), after
 * hovering the pad like a user: they sit on the pad and show on hover, as on a track header
 * (2533f415).
 */
export async function padButton(grid: Locator, name: string): Promise<Locator> {
  const button = grid.getByRole("button", { name, exact: true });
  await button.locator("xpath=ancestor::*[contains(@class, 'eth-drum-rack__slot')][1]").hover();
  await expect(button).toBeVisible();
  return button;
}

/** The device ids of the open device windows. */
async function windowIds(page: Page): Promise<string[]> {
  return page.locator("[data-device-window]").evaluateAll((els) => els.map((e) => e.getAttribute("data-device-window") ?? ""));
}

/**
 * Sets a kit `NumberField` (a `spinbutton`). The keyboard path by default: type the value
 * and press Enter. `{ dragSteps }` instead drags it up (positive) or down by that many
 * steps (the field's default 4 px per step), which is one undo step.
 */
export async function setNumberField(field: Locator, value: number | { dragSteps: number }): Promise<void> {
  if (typeof value === "number") {
    await field.fill(String(value));
    await field.press("Enter");
    return;
  }
  const page = field.page();
  const box = (await field.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  const dy = -value.dragSteps * 4;
  await page.mouse.move(x, y + dy / 2, { steps: 4 });
  await page.mouse.move(x, y + dy, { steps: 4 });
  await page.mouse.up();
}

/**
 * Opens the sample browser pane from the left rail, pinned (it keeps its space instead of
 * floating over the lanes), with the given root of its folder tree expanded (default: the
 * in-browser engine's bundled "factory" library; `null` expands nothing, e.g. on a remote
 * engine).
 */
export async function openLibrary(page: Page, location: string | null = "factory"): Promise<void> {
  const pin = page.getByRole("button", { name: "pin the library panel" });
  if (!(await pin.isVisible())) await page.getByRole("navigation", { name: "panels" }).getByRole("button", { name: "library", exact: true }).click();
  await pin.click();
  if (location !== null) await expandFolder(page, location);
}

/** The library's folder tree (the browser's Samples / MIDI view). */
export const libraryTree = (page: Page) => page.getByRole("tree", { name: "folders" });

/** Expands folders of the library tree, outermost first (open ones stay open). */
export async function expandFolder(page: Page, ...names: string[]): Promise<void> {
  for (const name of names) {
    const row = libraryTree(page).getByRole("treeitem", { name, exact: true });
    await expect(row).toBeVisible({ timeout: 20_000 });
    if ((await row.getAttribute("aria-expanded")) !== "true") await row.click();
    await expect(row).toHaveAttribute("aria-expanded", "true");
  }
}

/**
 * Shows an editor of the bottom drawer: "Automation", and the clip editors while a clip is
 * open ("piano roll" / "beat editor" for a MIDI clip, "Warp" for an audio one: 74b99d95). The
 * tempo map is the master's Tempo lane, the groove in the clip inspector and the pads in the
 * Drum Rack's window (e6d68591). The drawer is closed until a clip is opened; otherwise it is
 * opened from the command palette (⌘K → "Open editor drawer"), then the tab is picked.
 */
export async function openEditor(page: Page, tab: string): Promise<void> {
  const tabs = page.getByRole("tablist", { name: "editors" });
  if (!(await tabs.isVisible())) {
    await page.getByRole("button", { name: "command palette" }).click();
    await page.getByRole("combobox", { name: "search commands" }).fill("Open editor drawer");
    await page
      .getByRole("listbox", { name: "commands" })
      .getByRole("option", { name: /open editor drawer/ })
      .click();
  }
  await tabs.getByRole("tab", { name: tab, exact: true }).click();
  await expect(tabs.getByRole("tab", { name: tab, exact: true })).toHaveAttribute("aria-selected", "true");
}

/** The transport bar's Play button (exact: other buttons' labels may contain "play"). */
export const playButton = (page: Page): Locator => page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "play", exact: true });

/**
 * The transport bar's Play button while playing: it reads "Stop" (pressed). Not by its
 * "Stop (Space)" title: the kit tooltip holds a hovered element's title aside while shown.
 */
export const playingButton = (page: Page): Locator =>
  page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "stop", exact: true, pressed: true });

/**
 * The transport bar's Stop button (the Play button also reads "Stop" while playing). Stopped,
 * it returns to the selection cursor, then to the start.
 */
export const stopButton = (page: Page): Locator => page.getByTestId("transport-stop");

/**
 * Expands the inspector when it shows as its collapsed bar (a click on a clip or a lane never
 * expands a pane; a track header's click does).
 */
export async function showInspector(page: Page): Promise<void> {
  const bar = page.getByRole("button", { name: "expand the inspector panel" });
  if (await bar.isVisible()) await bar.click();
  await expect(page.getByTestId("inspector")).toBeVisible();
}

/**
 * Shows a page of the settings dialog: its category in the sidebar (its first page), or a page
 * of the open category by its name.
 */
export async function settingsPage(dialog: Locator, name: string): Promise<void> {
  await dialog.locator(".eth-settings__nav").getByRole("button", { name, exact: true }).click();
}

/** Opens Settings (the status bar's gear) on the Advanced category (its Engine page). */
export async function openAdvancedSettings(page: Page): Promise<void> {
  await page.getByRole("button", { name: "settings", exact: true }).click();
  await settingsPage(page.getByRole("dialog", { name: "settings" }), "advanced");
}

/**
 * Opens the engine server form ("Server address", "Token", Connect) in Settings > Advanced;
 * connecting closes the settings. While connected the top bar shows `remote-button`.
 */
export async function openEngineServer(page: Page): Promise<void> {
  await openAdvancedSettings(page);
  await expect(page.getByLabel("server address")).toBeVisible();
}

/**
 * A new project named `name`: "New project" (the open picker's, else the project menu's)
 * creates one named after today's date, then a click on its name renames it.
 */
export async function newProject(page: Page, name: string): Promise<void> {
  const screen = projectScreen(page);
  const before = await page.getByTestId("project-name").textContent();
  // Nothing open yet: the app shows the picker by itself once the engine is up.
  // Wait for it rather than opening the menu, which raced that opening under load.
  // (The name button is disabled while no project is open, in every language.)
  if (await page.getByTestId("project-name").isDisabled()) await expect(screen).toBeVisible({ timeout: 30_000 });
  if (await screen.isVisible()) await screen.getByTestId("welcome-new-project").click();
  else {
    await page.getByRole("button", { name: "projects" }).click();
    await page.getByRole("menu", { name: "project" }).getByRole("menuitem", { name: "new project" }).click();
  }
  await expect(screen).toHaveCount(0);
  await expect(page.getByTestId("project-name")).not.toHaveText(before ?? "");
  await renameProject(page, name);
}

/** Rename the open project in place (a click on its name in the top bar). */
export async function renameProject(page: Page, name: string): Promise<void> {
  await page.getByTestId("project-name").click();
  const field = page.getByTestId("project-name-input");
  await field.fill(name);
  await field.press("Enter");
  await expect(page.getByTestId("project-name")).toHaveText(name);
}

/**
 * The "Resolve missing items" dialog that lists what an opened or imported project is
 * missing (plugins, devices, audio files): waits for it and closes it ("Resolve later").
 */
export async function resolveLater(page: Page): Promise<void> {
  const dialog = page.getByRole("dialog", { name: "resolve missing items" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await dialog.getByRole("button", { name: "resolve later" }).click();
  await expect(dialog).toBeHidden();
}

/** The project picker (the welcome screen, "open project"), found the same way in every language. */
export const projectScreen = (page: Page): Locator => page.locator('[role="dialog"].eth-project-screen--welcome');

/** Clicks an item of the top bar's project menu ("New project", "Open project…", "Export…", …). */
export async function projectMenu(page: Page, item: string): Promise<void> {
  await page.getByRole("button", { name: "projects" }).click();
  await page.getByRole("menu", { name: "project" }).getByRole("menuitem", { name: item, exact: true }).click();
}

/**
 * Opens the project picker: the command palette's "open project…" (the project menu's
 * "open project ›" is a submenu of recent projects).
 */
export async function openProjectScreen(page: Page): Promise<Locator> {
  await paletteCommand(page, "open project…");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible();
  return screen;
}

/**
 * Picks `source` ("dawproject…", "reaper…", …) in the project menu's "Import from another DAW"
 * submenu (the file chooser follows). On the launch project screen, Escape first: the draft
 * behind it opens. Retries the hover and click (a re-render can close the submenu between them).
 */
export async function importFromAnotherDaw(page: Page, source: string): Promise<void> {
  if (await projectScreen(page).isVisible()) {
    await page.keyboard.press("Escape");
    await expect(projectScreen(page)).toHaveCount(0);
  }
  await page.getByRole("button", { name: "projects" }).click();
  await expect(async () => {
    await page.getByRole("menu", { name: "project" }).getByRole("menuitem", { name: "import from another DAW" }).hover();
    await page.getByRole("menu", { name: "import from another DAW" }).getByRole("menuitem", { name: source }).click({ timeout: 3000 });
  }).toPass({ timeout: 20_000 });
}

/** Runs a command palette entry by its label (⌘K, search, pick the matching option). */
export async function paletteCommand(page: Page, label: string): Promise<void> {
  await page.getByRole("button", { name: "command palette" }).click();
  await page.getByRole("combobox", { name: "search commands" }).fill(label);
  await page.getByRole("listbox", { name: "commands" }).getByRole("option", { name: label }).first().click();
}

/**
 * The app launches with no project open, on the project picker. Loads the app and
 * creates a new, empty project there (what most specs start from).
 */
export async function launch(page: Page, name = "Untitled"): Promise<void> {
  await page.goto("/");
  await createOnLaunch(page, name);
}

/** On the launch project screen (after `goto`/`reload`): create and open a new project. */
export async function createOnLaunch(page: Page, name = "Untitled"): Promise<void> {
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, name);
}

/**
 * On the launch project screen (after `reload`): open a stored project from Recents, by
 * name, or the most recent one. A reload of a project's URL has it open already.
 */
export async function openOnLaunch(page: Page, name?: string): Promise<void> {
  const screen = projectScreen(page);
  // A reload at /p/<slug>/<id> opens that project by itself (no picker): done when it is the
  // one asked for, else pick it from the project screen.
  if (/^\/p\/[^/]*\/[^/]+$/.test(new URL(page.url()).pathname)) {
    const label = page.getByTestId("project-name");
    await expect(label).not.toHaveText("no project", { timeout: 30_000 });
    if (!name || (await label.textContent())?.trim() === name) return;
    await openProjectScreen(page);
  }
  await expect(screen).toBeVisible({ timeout: 30_000 });
  const entry = name
    ? screen.getByRole("button", { name: `open ${name}`, exact: true })
    : screen
        .getByRole("list", { name: "stored projects" })
        .getByRole("button", { name: /^open / })
        .first();
  const label = (await entry.getAttribute("aria-label"))!.slice("open ".length);
  await entry.click();
  await expect(page.getByTestId("project-name")).toHaveText(label);
  await expect(screen).toHaveCount(0);
}

/**
 * A track header's button (`name`: its accessible name), after hovering the header like a
 * user: buttons that aren't "on" only take room while the header is hovered.
 */
export async function headerButton(page: Page, trackName: string, name: string): Promise<Locator> {
  const header = page.getByRole("group", { name: `${trackName} track` });
  await header.hover();
  const button = header.getByRole("button", { name });
  // It grows from zero width on hover: wait until it has room (a forced click needs a box).
  await expect(button).toBeVisible();
  return button;
}

// ---- Piano roll notes (drawn on a canvas, not elements) -------------------------------------

/** A note as the piano roll draws it: its box in page px and its state. */
export interface DrawnNote {
  id: string;
  pitch: number;
  x: number;
  y: number;
  width: number;
  height: number;
  selected: boolean;
  inactive: boolean;
  tone?: "root" | "in" | "out";
}

/**
 * The notes drawn on the piano roll's grid (`piano-roll-grid`), in paint order (selected last),
 * from the grid's scene hook (`ethScene`, see `noteLayout.ts`).
 */
export async function pianoRollNotes(grid: Locator): Promise<DrawnNote[]> {
  const box = (await grid.boundingBox())!;
  const drawn = await grid.evaluate((el) => {
    type Item = {
      kind: string;
      id: string;
      pitch: number;
      x: number;
      y: number;
      w: number;
      h: number;
      selected: boolean;
      inactive: boolean;
      tone?: "root" | "in" | "out";
    };
    const scene = (el as Element & { ethScene?: () => { notes: Item[] } }).ethScene?.();
    return (scene?.notes ?? [])
      .filter((i) => i.kind === "note")
      .map(({ id, pitch, x, y, w, h, selected, inactive, tone }) => ({ id, pitch, x, y, w, h, selected, inactive, tone }));
  });
  return drawn.map(({ x, y, w, h, ...rest }) => ({ ...rest, x: box.x + x, y: box.y + y, width: w, height: h }));
}

/** The drawn box (page px) of note `id` (fails when it is not drawn). */
export async function pianoRollNote(grid: Locator, id: string): Promise<DrawnNote> {
  const note = (await pianoRollNotes(grid)).find((n) => n.id === id);
  expect(note, `note ${id} is drawn`).toBeTruthy();
  return note!;
}

/** Click a drawn note (its centre), holding `modifiers`. */
export async function clickPianoRollNote(
  page: Page,
  note: Pick<DrawnNote, "x" | "y" | "width" | "height">,
  opts: { button?: "left" | "right"; modifiers?: Array<"Shift" | "Alt" | "ControlOrMeta"> } = {},
): Promise<void> {
  for (const m of opts.modifiers ?? []) await page.keyboard.down(m);
  await page.mouse.click(note.x + note.width / 2, note.y + note.height / 2, { button: opts.button });
  for (const m of opts.modifiers ?? []) await page.keyboard.up(m);
}
