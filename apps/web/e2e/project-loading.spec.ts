// A project still loading (issue #190): the engine's audio is suspended until every audio
// file is decoded, the transport says so (play, stop and record disabled, the space bar
// too, the progress shown), and the project can be explored meanwhile. Once loaded, it
// plays.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Project, TransportState } from "@/generated";
import { arrangerApi } from "./clips";
import { launch, openOnLaunch, playButton, playingButton, stopButton } from "./ui";

interface Handle {
  state(): { project: Project | null; transport: TransportState | null };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());

const FILES = 6;
const SECONDS = 28;

/** A stereo 16-bit 44.1 kHz WAV (decoded, then resampled to the engine's rate on load). */
function wav(hz: number): Buffer {
  const rate = 44100;
  const frames = rate * SECONDS;
  const bytes = frames * 4;
  const b = Buffer.alloc(44 + bytes);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + bytes, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(2, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 4, 28);
  b.writeUInt16LE(4, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(bytes, 40);
  for (let i = 0; i < frames; i++) {
    const v = Math.round(9000 * Math.sin((2 * Math.PI * hz * i) / rate));
    b.writeInt16LE(v, 44 + i * 4);
    b.writeInt16LE(v, 46 + i * 4);
  }
  return b;
}

test("while a project loads the transport waits, and the project can be explored", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, "Loading");
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (await chooser).setFiles(Array.from({ length: FILES }, (_, i) => ({ name: `tone ${i}.wav`, mimeType: "audio/wav", buffer: wav(110 * (i + 1)) })));
  await expect.poll(async () => Object.keys((await state(page)).project?.clips ?? {}).length, { timeout: 90_000 }).toBe(FILES);
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByRole("status", { name: "unsaved changes" })).toHaveCount(0, { timeout: 20_000 });

  // Reopen it: the audio files are decoded again.
  await page.reload();
  await openOnLaunch(page, "Loading");
  const loading = page.getByTestId("project-loading");
  await expect(loading).toBeVisible();
  // Its progress (the text is hidden in a compact top bar; the bar always says it).
  await expect(loading.getByRole("progressbar", { name: /loading audio files \d+\/6/ })).toBeVisible();
  await expect(playButton(page)).toBeDisabled();
  await expect(stopButton(page)).toBeDisabled();
  await expect(page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "record" })).toBeDisabled();
  expect((await state(page)).transport?.loading).toBeTruthy();
  // One loading indicator (#301): the spinner is on the status; the position keeps its
  // readout, dimmed, with no second "loading" label.
  await expect(loading.locator(".eth-tb__loading-spin")).toBeVisible();
  await expect(page.locator(".eth-tb__loading-spin")).toHaveCount(1);
  await expect(page.locator(".eth-tb__position[data-loading]")).toBeVisible();
  await expect(page.getByTestId("position-loading")).toHaveCount(0);
  // The tracks whose audio files load look unavailable, their controls inactive, the bar shown.
  // Each track is ready once its own file is decoded (#353), so look at one still loading; its
  // bar counts its own files (one here).
  const first = page.locator('[data-tour="track-header"][data-loading="true"]').first();
  await expect(first).toHaveClass(/eth-arr-header--loading/);
  await expect(first.getByRole("button", { name: /^mute / })).toBeDisabled();
  await expect(first.getByRole("button", { name: /^mute / })).toHaveAttribute("title", "still loading");
  await expect(first.getByRole("progressbar", { name: /loading audio files 0\/1/ })).toBeVisible();
  const filter = await first.locator(".eth-arr-header__main").evaluate((e) => getComputedStyle(e).filter);
  expect(filter).toMatch(/saturate/);
  if (process.env.ETH_SHOTS_DIR) await page.screenshot({ path: `${process.env.ETH_SHOTS_DIR}/project-loading.png` });
  // The space bar doesn't play either.
  await page.keyboard.press("Space");
  expect((await state(page)).transport?.playing).toBe(false);

  // Explorable meanwhile: the tracks and their clips are there.
  await expect(page.getByRole("group", { name: /^\d audio track$/ })).toHaveCount(FILES, { timeout: 10_000 });
  await expect.poll(async () => (await arrangerApi(page, "visibleClipIds")).length, { timeout: 10_000 }).toBe(FILES);
  // And editable: a rename lands at once.
  const header = page.getByRole("group", { name: "1 audio track" });
  await header.getByText("1 audio", { exact: true }).dblclick();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("renamed");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("group", { name: "renamed track" })).toBeVisible();
  expect((await state(page)).transport?.loading).toBeTruthy();

  // Loaded: the transport is back and plays.
  await expect(loading).toHaveCount(0, { timeout: 120_000 });
  expect((await state(page)).transport?.loading).toBeUndefined();
  // The tracks are back to normal, and so is the time.
  await expect(page.locator(".eth-arr-header--loading")).toHaveCount(0);
  await expect(page.getByRole("group", { name: "renamed track" }).getByRole("button", { name: /^mute / })).toBeEnabled();
  await expect(page.locator(".eth-tb__position[data-loading]")).toHaveCount(0);
  // (A compact top bar shows the time alone.)
  await expect(page.getByTestId("position-time")).toBeVisible();
  await expect(playButton(page)).toBeEnabled();
  await playButton(page).click();
  await expect(playingButton(page)).toBeVisible();
  await playingButton(page).click();
  await expect(playButton(page)).toBeVisible();

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
