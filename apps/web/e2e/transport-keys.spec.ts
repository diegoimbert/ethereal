// Transport keys on the web build (real wasm engine): Space always plays / stops, even with
// a clicked button focused (the button is not pressed again) but not while typing; a click on
// a lane sets the selection cursor and, stopped, the playhead; Space stops and returns there,
// Shift+Space plays too and stops in place;
// the Stop button, stopped, goes to the start and back to the selection.
//
// No sleeps: every step waits on UI or engine state (`window.__ether`, apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Project, TransportState } from "@/generated";
import { createTrack, launch, playButton, stopButton } from "./ui";

interface Handle {
  state(): { project: Project | null; transport: TransportState | null };
}

const transport = (page: Page) =>
  page.evaluate(() => {
    const t = (window as unknown as { __ether: Handle }).__ether.state().transport;
    return { playing: t?.playing ?? null, loop: t?.loop_enabled ?? null };
  });
const playing = (page: Page) => transport(page).then((t) => t.playing);
const position = (page: Page) => page.getByTestId("position-bars").textContent();
const toolbar = (page: Page) => page.getByRole("toolbar", { name: "transport" });

test("transport keys: Space always plays/stops, Shift+Space and Stop return to the selection", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  // A wide window: the top bar shows the bars position (compact, it shows the time).
  await page.setViewportSize({ width: 1600, height: 900 });
  await launch(page, `Transport keys ${Date.now()}`);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  const track = await createTrack(page, "Midi");

  // A mouse click on Loop toggles it once; the focus doesn't stay on it, and Space then plays
  // instead of pressing Loop again.
  const before = (await transport(page)).loop;
  await toolbar(page).getByRole("button", { name: "loop" }).click();
  await expect.poll(() => transport(page).then((t) => t.loop)).toBe(!before);
  await page.keyboard.press("Space");
  await expect.poll(() => playing(page)).toBe(true);
  expect((await transport(page)).loop).toBe(!before);
  // Even with the button focused by the keyboard, Space is the transport.
  await toolbar(page).getByRole("button", { name: "metronome" }).focus();
  await page.keyboard.press("Space");
  await expect.poll(() => playing(page)).toBe(false);

  // Typing in a text field keeps its spaces.
  const tempo = page.getByLabel("tempo");
  await tempo.focus();
  await page.keyboard.press("Space");
  expect(await playing(page)).toBe(false);
  await page.keyboard.press("Escape");

  // A click on the lane sets the selection cursor; stopped, the playhead goes there.
  const lane = page.locator(`[data-lane="${track.id}"]`);
  const box = (await lane.boundingBox())!;
  await lane.click({ position: { x: Math.round(box.width / 3), y: 30 } });
  await expect(page.getByTestId("insert-marker")).toBeVisible();
  await expect.poll(() => position(page)).not.toBe("1.1.1");
  const cursorAt = await position(page);

  // Play, then Space: stop and back to the selection.
  await page.keyboard.press("Space");
  await expect.poll(() => playing(page)).toBe(true);
  await expect.poll(() => position(page)).not.toBe(cursorAt);
  await page.keyboard.press("Space");
  await expect.poll(() => playing(page)).toBe(false);
  await expect.poll(() => position(page)).toBe(cursorAt);

  // Shift+Space plays too; stopping with it stays where playback is.
  await page.keyboard.press("Shift+Space");
  await expect.poll(() => playing(page)).toBe(true);
  await expect.poll(() => position(page)).not.toBe(cursorAt);
  await page.keyboard.press("Shift+Space");
  await expect.poll(() => playing(page)).toBe(false);
  expect(await position(page)).not.toBe(cursorAt);
  // Back to the selection for the Stop checks below.
  await page.keyboard.press("Space");
  await expect.poll(() => playing(page)).toBe(true);
  await page.keyboard.press("Space");
  await expect.poll(() => position(page)).toBe(cursorAt);

  // Stop while stopped: at the selection → the start; again → the selection.
  const stop = stopButton(page);
  await stop.click();
  await expect.poll(() => position(page)).toBe("1.1.1");
  await stop.click();
  await expect.poll(() => position(page)).toBe(cursorAt);

  expect(errors).toEqual([]);
});
