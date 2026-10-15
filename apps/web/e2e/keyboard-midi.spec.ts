// keyboard-midi: the computer MIDI keyboard against the real wasm engine (WasmTransport →
// controller Worker → AudioWorklet). A MIDI track with the Poly Synth, the top-bar toggle
// (auto-arms the selected MIDI track), then the home row plays it: a held A sounds on the
// track meter, releasing it goes silent again; Z/X and C/V move the indicator; a blur
// releases a held note (no stuck notes).
//
// `KEYBOARD_MIDI_SHOTS=<dir>` also saves the PR screenshots (dark 1440×900, plus light).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, launch, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null; armedTracks: string[] };
  meter(track: string): { peak: [number, number] } | null;
}

const shots = process.env.KEYBOARD_MIDI_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

const state = (page: Page) =>
  page.evaluate(() => {
    const s = (window as unknown as { __ether: Handle }).__ether.state();
    return { project: s.project, armed: s.armedTracks };
  });

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

test("keyboard-midi: the computer keyboard plays the armed track's instrument", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  // Nothing is open on launch (base-131): create the project on the launch screen.
  await launch(page, `Keys ${Date.now()}`);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });

  const midi = await createTrack(page, "Midi");
  await openDeviceTab(page, midi.name);
  await addDevice(page, "PolySynth");
  await expect.poll(async () => Object.values((await state(page)).project!.devices).some((d) => d.track === midi.id)).toBe(true);

  // On: the selected MIDI track is armed so it hears the keyboard.
  const toggle = page.getByRole("button", { name: "computer MIDI keyboard" });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await state(page)).armed).toEqual([midi.id]);
  const status = page.getByTestId("keyboard-midi-status");
  await expect(status).toHaveAccessibleName("octave C3, velocity 100");
  // Keys go to the page, not to the toggle button.
  await page.locator("body").focus();
  await expect.poll(() => peakOf(page, midi.id)).toBeLessThan(0.001);

  // A held "A" (C3) sounds on the track meter; releasing it goes silent again.
  await page.keyboard.down("KeyA");
  await expect(status).toHaveAttribute("data-playing", "true");
  await expect.poll(() => peakOf(page, midi.id), { timeout: 15_000 }).toBeGreaterThan(0.01);
  if (shots) await page.screenshot({ path: `${shots}/keyboard-midi-dark.png` });
  await page.keyboard.up("KeyA");
  await expect(status).not.toHaveAttribute("data-playing");
  await expect.poll(() => peakOf(page, midi.id), { timeout: 15_000 }).toBeLessThan(0.001);

  // Octave (Z/X) and velocity (C/V) controls.
  await page.keyboard.press("KeyX");
  await page.keyboard.press("KeyV");
  await expect(status).toHaveAccessibleName("octave C4, velocity 120");
  await page.keyboard.press("KeyZ");
  await page.keyboard.press("KeyC");
  await expect(status).toHaveAccessibleName("octave C3, velocity 100");

  // A blur while a key is held releases the note (no stuck note).
  await page.keyboard.down("KeyD");
  await expect.poll(() => peakOf(page, midi.id), { timeout: 15_000 }).toBeGreaterThan(0.01);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(status).not.toHaveAttribute("data-playing");
  await expect.poll(() => peakOf(page, midi.id), { timeout: 15_000 }).toBeLessThan(0.001);
  await page.keyboard.up("KeyD");

  // Off (M): the keys are left to the app again and play nothing.
  await page.keyboard.press("KeyM");
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await page.keyboard.down("KeyA");
  await page.waitForTimeout(300);
  expect(await peakOf(page, midi.id)).toBeLessThan(0.001);
  await page.keyboard.up("KeyA");

  if (shots) {
    await page.keyboard.press("KeyM");
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
    await page.keyboard.down("KeyF");
    await expect(status).toHaveAttribute("data-playing", "true");
    await page.screenshot({ path: `${shots}/keyboard-midi-light.png` });
    // The top bar (below the web's "Open on desktop" band).
    const bar = (await page.locator(".eth-shell__top").boundingBox())!;
    const topBar = { x: 0, y: 0, width: 1440, height: bar.y + bar.height + 8 };
    await page.screenshot({ path: `${shots}/keyboard-midi-topbar-light.png`, clip: topBar });
    await page.keyboard.up("KeyF");
    await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));
    await page.screenshot({ path: `${shots}/keyboard-midi-topbar-dark.png`, clip: topBar });
  }

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
