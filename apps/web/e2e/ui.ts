// Shared e2e gestures for the app's UI patterns. Import from here instead of re-deriving the
// gesture in each spec, so a UX change (a new flow, a kit control reworked) is fixed in one
// place. Clip gestures live in ./clips.ts.
import { expect, type Locator, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";

interface Handle {
  state(): { project: Project | null };
}

const tracksOf = (page: Page): Promise<Track[]> =>
  page.evaluate(() =>
    Object.values((window as unknown as { __ether: Handle }).__ether.state().project?.tracks ?? {}),
  );

/**
 * Adds a track the way a user does: "New track" puts a draft row in the arrangement, which
 * asks for the type in place ("Create MIDI track" / "Create audio track"). Resolves with the
 * new track once the controller has it.
 */
export async function createTrack(page: Page, kind: "Midi" | "Audio"): Promise<Track> {
  const before = new Set((await tracksOf(page)).map((t) => t.id));
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: kind === "Midi" ? "Create MIDI track" : "Create audio track" }).click();
  let created: Track | undefined;
  await expect
    .poll(async () => {
      created = (await tracksOf(page)).find((t) => !before.has(t.id) && t.kind === kind);
      return created !== undefined;
    })
    .toBe(true);
  return created!;
}

/**
 * Chooses an option of a kit `Select` like a user: opens the list (a `combobox` button) and
 * clicks the option. `select` is the combobox, or its accessible name (exact) within `scope`.
 * `option` is the option's label (exact) or `{ value }` for its value.
 */
export async function pickOption(
  scope: Page | Locator,
  select: string | Locator,
  option: string | { value: string },
): Promise<void> {
  const trigger = typeof select === "string" ? scope.getByRole("combobox", { name: select, exact: true }) : select;
  const page = "page" in scope ? scope.page() : scope;
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  // The list this trigger controls (portaled to <body>; another may still be animating out).
  const listbox = page.locator(`[id="${await trigger.getAttribute("aria-controls")}"]`);
  const item =
    typeof option === "string"
      ? listbox.getByRole("option", { name: option, exact: true })
      : listbox.locator(`[role="option"][data-value="${option.value}"]`);
  await item.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

/**
 * Selects a track the way a user does: a click on its header card, at its left edge. The
 * card's center lands on its mute toggle, and the name can't be clicked: at the default
 * header width it is squeezed to 0 px (an app layout bug reported by base-46).
 */
export async function selectTrack(page: Page, trackName: string): Promise<void> {
  const header = page.getByRole("group", { name: `${trackName} track` });
  const box = (await header.boundingBox())!;
  await header.click({ position: { x: 6, y: box.height / 2 } });
}

/**
 * Shows a track's devices: selecting the track opens the inspector with its device chain.
 * Resolves once the chain's "Add device" picker is there.
 */
export async function openDeviceTab(page: Page, trackName: string): Promise<void> {
  await selectTrack(page, trackName);
  await expect(page.getByRole("combobox", { name: "Add device" })).toBeVisible();
}

/** Inserts a built-in device (by its type, e.g. "Compressor") from the chain's "Add device" picker. */
export async function addDevice(page: Page, type: string): Promise<void> {
  await pickOption(page, "Add device", { value: type });
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
 * floating over the lanes), on the given location tab (default: the in-browser engine's
 * bundled library; `null` keeps the current one, e.g. on a remote engine).
 */
export async function openLibrary(page: Page, location: string | null = "Browser library"): Promise<void> {
  const pin = page.getByRole("button", { name: "Pin Library" });
  if (!(await pin.isVisible())) await page.getByRole("navigation", { name: "Panels" }).getByRole("button", { name: "Library", exact: true }).click();
  await pin.click();
  if (location !== null) await page.getByRole("tablist", { name: "Locations" }).getByRole("tab", { name: location }).click();
}

/**
 * Shows an editor of the bottom drawer ("Piano Roll", "Warp", "Automation", "Tempo",
 * "Groove", "Drum Rack"). The drawer is closed until a clip is opened; otherwise it is
 * opened from the command palette (⌘K → "Open editor drawer"), then the tab is picked.
 */
export async function openEditor(page: Page, tab: string): Promise<void> {
  const tabs = page.getByRole("tablist", { name: "Editors" });
  if (!(await tabs.isVisible())) {
    await page.getByRole("button", { name: "Command palette" }).click();
    await page.getByRole("combobox", { name: "Search commands" }).fill("Open editor drawer");
    await page.getByRole("listbox", { name: "Commands" }).getByRole("option", { name: /Open editor drawer/ }).click();
  }
  await tabs.getByRole("tab", { name: tab, exact: true }).click();
  await expect(tabs.getByRole("tab", { name: tab, exact: true })).toHaveAttribute("aria-selected", "true");
}

/** The transport bar's Play button (exact: "Add marker at playhead" also contains "Play"). */
export const playButton = (page: Page): Locator =>
  page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Play", exact: true });

/** Opens Settings (top-bar gear) on the Advanced tab (base-115, docs/SHARING.md §8.6). */
async function openAdvancedSettings(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("dialog", { name: "Settings" }).getByRole("tab", { name: "Advanced" }).click();
}

/**
 * Opens the relay join dialog ("Relay address", "Session", "Your name", "Token", Join): it
 * moved from the top bar to Settings > Advanced > Relay session (the Share button took its
 * place). During a session the top bar still shows the relay bar (`collab-button`).
 */
export async function openRelayJoin(page: Page): Promise<void> {
  await openAdvancedSettings(page);
  await page.getByRole("button", { name: "Join a relay session…" }).click();
  // The settings close first (their exit animation keeps them in the DOM for a moment).
  await expect(page.getByTestId("settings-advanced")).toHaveCount(0);
  await expect(page.getByLabel("Relay address")).toBeVisible();
}

/**
 * Opens the engine server form ("Server address", "Token", Connect) in Settings > Advanced;
 * connecting closes the settings. While connected the top bar shows `remote-button`.
 */
export async function openEngineServer(page: Page): Promise<void> {
  await openAdvancedSettings(page);
  await expect(page.getByLabel("Server address")).toBeVisible();
}

/**
 * Creates and opens a new, empty project from the project screen (Projects button, or the
 * screen already shown on launch, base-131).
 */
export async function newProject(page: Page, name: string): Promise<void> {
  const screen = page.getByRole("dialog", { name: "Projects" });
  if (!(await screen.isVisible())) await page.getByRole("button", { name: "Projects" }).click();
  await screen.getByRole("button", { name: "New project" }).click();
  await screen.getByLabel("New project name").fill(name);
  await screen.getByRole("button", { name: "Create" }).click();
  await expect(page.getByTestId("project-name")).toHaveText(name);
  await expect(screen).toHaveCount(0);
}

/** The project screen (a dialog named "Projects"). */
export const projectScreen = (page: Page): Locator => page.getByRole("dialog", { name: "Projects" });

/**
 * base-131: the app launches with no project open, on the project screen. Loads the app and
 * creates a new, empty project there (what most specs start from).
 */
export async function launch(page: Page, name = "Untitled"): Promise<void> {
  await page.goto("/");
  await createOnLaunch(page, name);
}

/** On the launch project screen (after `goto`/`reload`): create and open a new project. */
export async function createOnLaunch(page: Page, name = "Untitled"): Promise<void> {
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: "New project" }).click();
  await screen.getByLabel("New project name").fill(name);
  await screen.getByRole("button", { name: "Create" }).click();
  await expect(page.getByTestId("project-name")).toHaveText(name);
  await expect(screen).toHaveCount(0);
}

/**
 * On the launch project screen (after `reload`): open a stored project from Recents, by
 * name, or the most recent one.
 */
export async function openOnLaunch(page: Page, name?: string): Promise<void> {
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  const entry = name
    ? screen.getByRole("button", { name: `Open ${name}`, exact: true })
    : screen.getByRole("list", { name: "Stored projects" }).getByRole("button", { name: /^Open / }).first();
  const label = (await entry.getAttribute("aria-label"))!.slice("Open ".length);
  await entry.click();
  await expect(page.getByTestId("project-name")).toHaveText(label);
  await expect(screen).toHaveCount(0);
}
