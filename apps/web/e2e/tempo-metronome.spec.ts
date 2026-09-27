// Tempo map + metronome on the web build, through the UI, against the real engine
// (WasmTransport → controller Worker → AudioWorklet).
//
// Metronome settings popover: switch on (the transport bar button follows), volume, sound;
// play and stop with the click on → Tempo tab: add a tempo point and a time-signature
// change by double-clicking the lanes, set the signature to 7/8 → arrangement ruler: add a
// tempo change from its context menu → undo.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { newProject } from "./projects";

interface Handle {
  state(): { project: Project | null; transport: { playing: boolean } | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const playing = (page: Page): Promise<boolean> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().transport?.playing ?? false);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const tempoPoints = async (page: Page) => Object.values((await doc(page)).tempo_points).sort((a, b) => a.time - b.time);
const signatures = async (page: Page) => Object.values((await doc(page)).time_signatures).sort((a, b) => a.time - b.time);

test("tempo-metronome: metronome settings, tempo map editing", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await newProject(page, `Tempo ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(1);

  // --- Metronome settings -------------------------------------------------------------------
  await page.getByRole("button", { name: "Metronome settings" }).click();
  await page.getByRole("switch", { name: "Metronome" }).click();
  await expect.poll(async () => (await doc(page)).settings.metronome).toBe(true);
  await expect(page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Metronome" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  const volume = page.getByRole("spinbutton", { name: "Metronome volume" });
  await volume.fill("-12");
  await volume.press("Enter");
  await expect.poll(async () => (await doc(page)).settings.metronome_volume).toBe(-12);
  await page.getByLabel("Metronome sound").selectOption("Wood");
  await expect.poll(async () => (await doc(page)).settings.metronome_sound).toBe("Wood");
  await page.keyboard.press("Escape");

  // Play a moment with the click on, then stop.
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect.poll(() => playing(page)).toBe(true);
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true }).first().click();
  await expect.poll(() => playing(page)).toBe(false);

  // Toggle it off again from the transport bar: the popover's switch follows.
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Metronome" }).click();
  await expect.poll(async () => (await doc(page)).settings.metronome).toBe(false);

  // --- Tempo tab ----------------------------------------------------------------------------
  await page.getByRole("tablist", { name: "Detail view" }).getByRole("tab", { name: "Tempo" }).click();
  const lane = page.getByTestId("tempo-lane-svg");
  await expect(lane).toBeVisible();
  const box = (await lane.boundingBox())!;
  await lane.dblclick({ position: { x: 12 * 8, y: box.height * 0.25 } });
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(2);
  const added = (await tempoPoints(page))[1]!;
  expect(added.time).toBeCloseTo(8, 6);
  const bpm = page.getByRole("spinbutton", { name: "Tempo point BPM" });
  await bpm.fill("90");
  await bpm.press("Enter");
  await expect.poll(async () => (await doc(page)).tempo_points[added.id]?.bpm).toBe(90);
  await page.getByLabel("Tempo curve").selectOption("Linear");
  await expect.poll(async () => (await doc(page)).tempo_points[added.id]?.curve).toBe("Linear");

  // A 7/8 change on bar 3 (beat 8).
  await page.getByTestId("signature-lane").dblclick({ position: { x: 12 * 8 + 2, y: 6 } });
  await expect.poll(async () => (await signatures(page)).length).toBe(2);
  const beats = page.getByRole("spinbutton", { name: "Beats per bar" });
  await beats.fill("7");
  await beats.press("Enter");
  await expect.poll(async () => (await signatures(page))[1]!.signature.numerator).toBe(7);
  await page.getByLabel("Beat unit").selectOption("8");
  await expect.poll(async () => (await signatures(page))[1]!.signature).toEqual({ numerator: 7, denominator: 8 });

  // --- Arrangement ruler: add a tempo change from the context menu, then undo ---------------
  const ruler = page.locator(".eth-arr__ruler");
  await expect(ruler.locator(`[data-tempo-point="${added.id}"]`)).toBeVisible();
  const rulerBox = (await ruler.boundingBox())!;
  await ruler.click({ button: "right", position: { x: rulerBox.width * 0.6, y: rulerBox.height / 2 } });
  await page.getByRole("menuitem", { name: "Add Tempo Change Here" }).click();
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(3);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(2);

  expect(errors).toEqual([]);
});
