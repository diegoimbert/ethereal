// keymap on the web build (real wasm engine + controller, OPFS user library): the editor
// opens from the palette, rebinding works at once (a new chord runs the action, a removed one
// no longer does), conflicts show, the keymap survives a reload (stored by the engine in the
// user library), the Ableton-like preset adds its chords without breaking the defaults, and
// the cheat sheet prints.
import { expect, test, type Page } from "@playwright/test";
import type { Project, TransportState } from "@/generated";
import { createTrack, playButton, launch, openOnLaunch } from "./ui";

interface Handle {
  state(): { project: Project | null; transport: TransportState | null };
}
const state = (page: Page) =>
  page.evaluate(() => {
    const s = (window as unknown as { __ether: Handle }).__ether.state();
    return { metronome: s.project?.settings.metronome ?? null, loop: s.transport?.loop_enabled ?? null, playing: s.transport?.playing ?? null, tracks: Object.keys(s.project?.tracks ?? {}).length };
  });

const editor = (page: Page) => page.getByRole("dialog", { name: "Keyboard shortcuts" });
const row = (page: Page, id: string) => editor(page).locator(`[data-action="${id}"]`);

const paletteInput = (page: Page) => page.getByRole("combobox", { name: "Search commands" });

/** Open the palette once any previous one has closed (⌘K on an open palette toggles it shut). */
async function openPalette(page: Page) {
  await expect(paletteInput(page)).toBeHidden();
  await page.keyboard.press("ControlOrMeta+k");
  await expect(paletteInput(page)).toBeVisible();
}

async function openEditor(page: Page) {
  await openPalette(page);
  await page.getByRole("combobox", { name: "Search commands" }).fill("keyboard shortcuts");
  await page.keyboard.press("Enter");
  await expect(editor(page)).toBeVisible();
}

async function closeEditor(page: Page) {
  await editor(page).getByRole("button", { name: "Done" }).click();
  // Gone after its exit animation (focus leaves it).
  await expect(page.locator(".eth-keymap")).toHaveCount(0);
}

/** Record `chord` (Playwright syntax) as an extra shortcut of action `id`. */
async function addChord(page: Page, id: string, chord: string) {
  await row(page, id).getByRole("button", { name: /Add a shortcut/ }).click();
  await expect(row(page, id).getByTestId("keymap-recording")).toBeVisible();
  await page.keyboard.press(chord);
  await expect(row(page, id).getByTestId("keymap-pending")).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(row(page, id).getByTestId("keymap-pending")).toHaveCount(0);
}

test("keymap: rebind, conflicts, persistence, Ableton-like preset, cheat sheet", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Keymap ${Date.now()}`);
  await createTrack(page, "Midi");

  // Defaults unchanged: Space plays/stops.
  await page.locator("body").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Space");
  await expect.poll(async () => (await state(page)).playing).toBe(true);
  await page.keyboard.press("Space");
  await expect.poll(async () => (await state(page)).playing).toBe(false);
  // The old lenient Shift+Space is kept as an alias.
  await page.keyboard.press("Shift+Space");
  await expect.poll(async () => (await state(page)).playing).toBe(true);
  await page.keyboard.press("Shift+Space");
  await expect.poll(async () => (await state(page)).playing).toBe(false);

  // Bind the metronome to Alt+M; move Play/Stop from Space to P.
  await openEditor(page);
  await addChord(page, "transport.metronome", "Alt+m");
  await expect(row(page, "transport.metronome").locator("kbd")).toHaveText(["Alt+M"]);
  await addChord(page, "transport.play", "p");
  await row(page, "transport.play").getByRole("button", { name: "Remove Space from Play / Stop" }).click();
  await row(page, "transport.play").getByRole("button", { name: "Remove ⇧Space from Play / Stop" }).click();
  await expect(row(page, "transport.play").locator("kbd")).toHaveText(["P"]);

  // A conflict shows on both rows while it lasts.
  await addChord(page, "transport.loop", "ControlOrMeta+d");
  await expect(row(page, "edit.duplicate").getByRole("note")).toContainText("is also Loop on / off");
  await row(page, "transport.loop").getByRole("button", { name: /Remove Ctrl\+D from Loop/ }).click();
  await expect(editor(page).getByRole("note")).toHaveCount(0);
  await closeEditor(page);

  const before = await state(page);
  await page.keyboard.press("Alt+m");
  await expect.poll(async () => (await state(page)).metronome).toBe(!before.metronome);
  await page.keyboard.press("Space");
  await page.waitForTimeout(300);
  expect((await state(page)).playing).toBe(false);
  await page.keyboard.press("p");
  await expect.poll(async () => (await state(page)).playing).toBe(true);
  await page.keyboard.press("p");
  await expect.poll(async () => (await state(page)).playing).toBe(false);
  // The transport bar's tooltip follows.
  await expect(playButton(page)).toHaveAttribute("title", "Play (P)");

  // Stored by the engine in the user library: a reload keeps it.
  await page.reload();
  // base-131: nothing opens on launch; reopen the project from Recents.
  await openOnLaunch(page);
  await expect(playButton(page)).toHaveAttribute("title", "Play (P)");
  await openEditor(page);
  await expect(row(page, "transport.metronome").locator("kbd")).toHaveText(["Alt+M"]);

  // Ableton-like: adds Mod+L (loop) and F9 (record) on top of the defaults; Mod+J still
  // toggles the editor drawer, and the user's own changes stay.
  await editor(page).getByRole("combobox", { name: "Keymap preset" }).click();
  await page.getByRole("option", { name: "Ableton-like" }).click();
  await expect(row(page, "view.editor").locator("kbd")).toHaveText(["Ctrl+J", "Alt+Ctrl+L"]);
  await expect(row(page, "transport.loop").locator("kbd")).toHaveText(["Ctrl+L"]);
  await expect(row(page, "transport.play").locator("kbd")).toHaveText(["P"]);
  await closeEditor(page);
  const loop = (await state(page)).loop;
  await page.keyboard.press("ControlOrMeta+l");
  await expect.poll(async () => (await state(page)).loop).toBe(!loop);

  // The palette shows the keymap's chords.
  await openPalette(page);
  await page.getByRole("combobox", { name: "Search commands" }).fill("metronome");
  await expect(page.getByRole("option", { name: /metronome/i }).first().locator("kbd")).toHaveText("Alt+M");
  await page.keyboard.press("Escape");

  // Cheat sheet: printing renders it (print() stubbed).
  await page.evaluate(() => {
    (window as unknown as { printed: number }).printed = 0;
    window.print = () => void ((window as unknown as { printed: number }).printed += 1);
  });
  await openPalette(page);
  await page.getByRole("combobox", { name: "Search commands" }).fill("print keyboard");
  await expect(page.getByRole("option", { name: /print keyboard/i }).first()).toBeVisible();
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => (window as unknown as { printed: number }).printed)).toBe(1);
  const sheet = page.getByTestId("keymap-cheat-sheet");
  await expect(sheet).toContainText("Ableton-like preset, customized");
  await expect(sheet).toContainText("Alt+M");

  // Back to the defaults.
  await page.evaluate(() => window.dispatchEvent(new Event("afterprint")));
  await openEditor(page);
  await editor(page).getByRole("button", { name: /Reset all/ }).click();
  await expect(row(page, "transport.play").locator("kbd")).toHaveText(["Space", "⇧Space"]);
  await closeEditor(page);
  expect(errors).toEqual([]);
});
