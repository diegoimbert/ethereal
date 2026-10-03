// Audio streaming on the web build (audio-streaming, CONTRACTS.md §13.1): a media longer
// than 30 s is not decoded into memory; the engine Worker reads it from OPFS by byte ranges
// and ships chunks to the worklet's cache. Import a 40 s file → it plays → click mid-clip
// (locate) → it plays from there → no stream underrun is reported and no engine error.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { newProject, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Engine {
  onMessages(l: (json: string) => void): void;
}

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

const SECONDS = 40;

/** A stereo 16-bit 48 kHz WAV: two sines (left 220 Hz, right 330 Hz). */
function longWav(): Buffer {
  const rate = 48000;
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
    b.writeInt16LE(Math.round(12000 * Math.sin((2 * Math.PI * 220 * i) / rate)), 44 + i * 4);
    b.writeInt16LE(Math.round(9000 * Math.sin((2 * Math.PI * 330 * i) / rate)), 46 + i * 4);
  }
  return b;
}

/** Seconds shown by the transport bar (`m:ss.xxx`). */
async function positionSeconds(page: Page): Promise<number> {
  const text = (await page.getByTestId("position-time").textContent()) ?? "";
  const m = /(\d+):(\d+(?:\.\d+)?)/.exec(text);
  return m ? Number(m[1]) * 60 + Number(m[2]) : NaN;
}

test("a long file streams from OPFS: plays and locates without underruns", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  // Count `MediaEvent::StreamUnderruns` and engine warnings from the server messages.
  await page.evaluate(() => {
    const w = window as unknown as { __etherEngine: Engine; __stream: { underruns: number; engineErrors: string[] } };
    w.__stream = { underruns: 0, engineErrors: [] };
    w.__etherEngine.onMessages((json) => {
      for (const m of json.matchAll(/"StreamUnderruns","count":(\d+)/g)) w.__stream.underruns += Number(m[1]);
      for (const m of json.matchAll(/"message":"(audio engine:[^"]*)"/g)) w.__stream.engineErrors.push(m[1]!);
    });
  });
  const stream = () =>
    page.evaluate(() => (window as unknown as { __stream: { underruns: number; engineErrors: string[] } }).__stream);

  await newProject(page, "Streaming");
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (await chooser).setFiles([{ name: "Long.wav", mimeType: "audio/wav", buffer: longWav() }]);
  await expect.poll(async () => Object.keys((await project(page)).clips).length, { timeout: 30_000 }).toBe(1);
  const p = await project(page);
  const media = Object.values(p.media)[0]!;
  expect(media.frames).toBe(48000 * SECONDS);
  const clip = Object.values(p.clips)[0]!;
  const track = clip.track;
  // The waveform comes from the one decode pass (peaks only).
  const clipEl = page.locator(`[data-clip-id="${clip.id}"]`);
  await expect(clipEl).toBeVisible();

  // Play from the start: the track sounds and time advances.
  await playButton(page).click();
  await expect.poll(() => peakOf(page, track), { timeout: 20_000 }).toBeGreaterThan(0.1);
  await expect.poll(() => positionSeconds(page), { timeout: 20_000 }).toBeGreaterThan(2);
  await page.getByRole("button", { name: "Stop", description: "Stop (Space)" }).click();

  // Locate mid-clip: a click on empty arrangement space (below the track) moves the playhead.
  const area = page.locator(".eth-arr__drop-area");
  const box = (await area.boundingBox())!;
  const clipBox = (await clipEl.boundingBox())!;
  const view = page.viewportSize()!;
  const x = clipBox.x + (Math.min(clipBox.x + clipBox.width, view.width) - clipBox.x) * 0.7 - box.x;
  await area.click({ position: { x, y: Math.min(10, box.height / 2) } });
  await expect.poll(() => positionSeconds(page)).toBeGreaterThan(1);
  const from = await positionSeconds(page);
  await playButton(page).click();
  await expect.poll(() => positionSeconds(page), { timeout: 20_000 }).toBeGreaterThan(from + 1.5);
  await expect.poll(() => peakOf(page, track), { timeout: 20_000 }).toBeGreaterThan(0.1);
  await page.getByRole("button", { name: "Stop", description: "Stop (Space)" }).click();

  const s = await stream();
  expect(s.underruns).toBe(0);
  expect(s.engineErrors).toEqual([]);
  expect(errors).toEqual([]);
});
