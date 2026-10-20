// Tempo map + metronome on the web build, through the UI, against the real engine
// (WasmTransport → controller Worker → AudioWorklet).
//
// Metronome settings popover: switch on (the transport bar button follows), volume, sound;
// play and stop with the click on → the master's Tempo lane (d5d2e4f9; the Tempo tab is
// gone): add a tempo point by double-clicking it, drag it to a new BPM, ramp it from its menu
// → a time-signature change from the ruler's menu, made 7/8 from its chip's menu →
// arrangement ruler: add a tempo change from its context menu → undo.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { headerButton, launch, newProject, pickOption, playButton, setNumberField } from "./ui";

interface Handle {
  state(): { project: Project | null; transport: { playing: boolean } | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const playing = (page: Page): Promise<boolean> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().transport?.playing ?? false);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const tempoPoints = async (page: Page) => Object.values((await doc(page)).tempo_points).sort((a, b) => a.time - b.time);

// WORKAROUND for an app layout bug (reported by for the user to fix): at the
// default 1280 px width the transport bar's right side (.eth-tb__side--end: undo/redo, CPU,
// engine status) overflows onto the Loop/Metronome buttons and takes their clicks. A wider
// window keeps them clickable. Drop this once the transport bar fits at 1280 px.
test.use({ viewport: { width: 1600, height: 900 } });

const signatures = async (page: Page) => Object.values((await doc(page)).time_signatures).sort((a, b) => a.time - b.time);

test("tempo-metronome: metronome settings, tempo map editing", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await newProject(page, `Tempo ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(1);

  // --- Metronome settings -------------------------------------------------------------------
  await page.getByRole("button", { name: "metronome settings" }).click();
  await page.getByRole("switch", { name: "metronome" }).click();
  await expect.poll(async () => (await doc(page)).settings.metronome).toBe(true);
  await expect(page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "metronome" })).toHaveAttribute("aria-pressed", "true");
  const volume = page.getByRole("spinbutton", { name: "metronome volume" });
  await setNumberField(volume, -12);
  await expect.poll(async () => (await doc(page)).settings.metronome_volume).toBe(-12);
  await pickOption(page, "metronome sound", { value: "Wood" });
  await expect.poll(async () => (await doc(page)).settings.metronome_sound).toBe("Wood");
  await page.keyboard.press("Escape");

  // Play a moment with the click on, then stop.
  await page.getByRole("button", { name: "play", exact: true }).click();
  await expect.poll(() => playing(page)).toBe(true);
  await page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "stop", exact: true }).first().click();
  await expect.poll(() => playing(page)).toBe(false);

  // Toggle it off again from the transport bar: the popover's switch follows.
  await page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "metronome" }).click();
  await expect.poll(async () => (await doc(page)).settings.metronome).toBe(false);

  // --- The master's Tempo lane (its first automation lane) -----------------------------------
  await (await headerButton(page, "master", "show automation of master")).click();
  const lane = page.getByTestId("tempo-automation-lane").getByTestId("tempo-lane-svg");
  await expect(lane).toBeVisible();
  // The arrangement's default zoom: 24 px per beat from the lane's left edge (beat 0).
  const pxPerBeat = 24;
  const box = (await lane.boundingBox())!;
  await lane.dblclick({ position: { x: pxPerBeat * 8 + 3, y: box.height * 0.25 } });
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(2);
  const added = (await tempoPoints(page))[1]!;
  expect(added.time).toBeCloseTo(8, 6);
  // Drag it down: a lower, whole BPM (one undo step).
  const point = lane.locator(`[data-point="${added.id}"]`);
  const pb = (await point.boundingBox())!;
  await page.mouse.move(pb.x + pb.width / 2, pb.y + pb.height / 2);
  await page.mouse.down();
  await page.mouse.move(pb.x + pb.width / 2, pb.y + pb.height / 2 + 10, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await doc(page)).tempo_points[added.id]!.bpm).toBeLessThan(added.bpm);
  const bpm = (await doc(page)).tempo_points[added.id]!.bpm;
  expect(Number.isInteger(bpm)).toBe(true);
  expect((await doc(page)).tempo_points[added.id]!.time).toBeCloseTo(8, 6);
  // Ramp to the next point (its menu).
  await point.click({ button: "right" });
  await page.getByRole("menuitem", { name: "ramp to next point" }).click();
  await expect.poll(async () => (await doc(page)).tempo_points[added.id]?.curve).toBe("Linear");

  // A 7/8 change on bar 3 (beat 8): the ruler's menu adds it, its chip's menu sets it.
  const ruler = page.locator(".eth-arr__ruler");
  const rulerBox = (await ruler.boundingBox())!;
  await ruler.click({ button: "right", position: { x: box.x - rulerBox.x + pxPerBeat * 8 + 2, y: rulerBox.height / 2 } });
  await page.getByRole("menuitem", { name: "add time signature change here" }).click();
  await expect.poll(async () => (await signatures(page)).length).toBe(2);
  const sig = (await signatures(page))[1]!;
  expect(sig.time).toBe(8);
  await page.getByTestId("marker-lane").locator(`[data-signature="${sig.id}"]`).click({ button: "right" });
  await page.getByRole("menuitem", { name: "7/8", exact: true }).click();
  await expect.poll(async () => (await signatures(page))[1]!.signature).toEqual({ numerator: 7, denominator: 8 });

  // --- Arrangement ruler: add a tempo change from the context menu, then undo ---------------
  // The tempo map's chips sit in the marker lane; right-click on the ruler still adds.
  await expect(page.getByTestId("marker-lane").locator(`[data-tempo-point="${added.id}"]`)).toBeVisible();
  await ruler.click({ button: "right", position: { x: rulerBox.width * 0.6, y: rulerBox.height / 2 } });
  await page.getByRole("menuitem", { name: "add tempo change here" }).click();
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(3);
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(async () => (await tempoPoints(page)).length).toBe(2);

  expect(errors).toEqual([]);
});
