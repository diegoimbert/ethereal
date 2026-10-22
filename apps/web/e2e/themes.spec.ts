// Themes (Settings > Appearance): the built-in theme cards restyle the whole app at once (the
// tokens, and the arranger's WebGL clips), and the choice survives a reload.
//
// `UX_SHOTS=<dir>` also writes the PR screenshots: the arrangement and the settings in each
// built-in theme.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { clipPart, pixelAt } from "./clips";
import { installProbe, send, ulid } from "./perfProject";
import { launch, openOnLaunch, settingsPage } from "./ui";

const shots = process.env.UX_SHOTS;

test.use({ viewport: { width: 1440, height: 900 } });

/** The built-in themes: id, card name, background. */
const THEMES = [
  ["dark", "dark", "#16181e"],
  ["obsidian", "obsidian", "#0a0a0c"],
  ["ember", "ember", "#191513"],
  ["tide", "tide", "#0e1521"],
  ["dusk", "dusk", "#24232b"],
  ["light", "light", "#eceef2"],
] as const;

const token = (page: Page, name: string): Promise<string> => page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(), name);

/** Two MIDI tracks with a few clips (something for the arranger to draw). */
async function scene(page: Page): Promise<void> {
  await installProbe(page);
  for (const [t, name] of [
    [0, "Keys"],
    [1, "Pads"],
  ] as const) {
    const track = ulid(1, t);
    await send(page, { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name, color: null, parent: null, before: null } });
    for (let c = 0; c < 3; c++) {
      const id = ulid(3, t * 10 + c);
      await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track, start: c * 6 + t * 2, length: 4, name: null } });
      await send(page, {
        domain: "Note",
        command: {
          type: "Add",
          clip: id,
          notes: Array.from({ length: 8 }, (_, k) => ({
            id: ulid(4, t * 1000 + c * 10 + k),
            pitch: 55 + ((k * 5) % 12),
            velocity: 100,
            start: k / 2,
            duration: 0.4,
          })),
        },
      });
    }
  }
}

async function openAppearance(page: Page): Promise<Locator> {
  await page.getByRole("button", { name: "settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "settings" });
  await settingsPage(dialog, "appearance");
  await expect(dialog.getByTestId("settings-appearance")).toBeVisible();
  return dialog;
}

async function closeSettings(page: Page): Promise<void> {
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "settings" })).toBeHidden();
}

const distance = (a: readonly number[], b: readonly number[]) => Math.max(...a.map((v, i) => Math.abs(v - b[i]!)));

test("built-in themes restyle the app and the arranger live, and are remembered", async ({ page }) => {
  test.setTimeout(150_000);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await launch(page, "Themes");
  await scene(page);
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "dark");
  const clip = await clipPart(page, ulid(3, 0), "body");
  const sample = () => pixelAt(page, Math.round(clip.x + clip.width * 0.9), Math.round(clip.y + clip.height - 3));
  await expect.poll(sample).not.toEqual([0, 0, 0]);

  const seen: Array<[number, number, number]> = [];
  for (const [id, name, bg] of THEMES) {
    const dialog = await openAppearance(page);
    const card = dialog.getByRole("button", { name, exact: true });
    await card.click();
    await expect(card).toHaveAttribute("aria-pressed", "true");
    await expect(html).toHaveAttribute("data-theme-id", id);
    await expect(html).toHaveAttribute("data-theme", id === "light" ? "light" : "dark");
    expect(await token(page, "--eth-color-bg")).toBe(bg);
    if (shots) await dialog.screenshot({ path: `${shots}/theme-${id}-settings.png` });
    await closeSettings(page);
    // The WebGL clips are redrawn in the new theme's colors.
    let px: [number, number, number] = [0, 0, 0];
    await expect
      .poll(async () => {
        px = await sample();
        return seen.every((s) => distance(s, px) > 2);
      })
      .toBe(true);
    seen.push(px);
    if (shots) {
      await page.mouse.move(0, 0);
      await page.screenshot({ path: `${shots}/theme-${id}.png` });
    }
  }

  // Remembered on this device.
  const dialog = await openAppearance(page);
  await dialog.getByRole("button", { name: "ember", exact: true }).click();
  await closeSettings(page);
  await page.reload();
  await openOnLaunch(page, "Themes");
  await expect(html).toHaveAttribute("data-theme-id", "ember");
  expect(await token(page, "--eth-color-accent")).toBe("#f0a35e");
});
