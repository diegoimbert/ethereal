// Complex warp on the web build keeps pitch (web-stretch: the pure-Rust Signalsmith Stretch
// in the AudioWorklet), through the UI, against the real engine.
//
// An 8 s, 440 Hz sine on an audio track, a Tuner after it. Warp on (the controller's BPM
// stub reads 8 s as 16 beats at 120 BPM), project tempo 80 BPM: the clip plays 1.5x
// slower. Complex: the tuner reads 440 Hz (pitch kept). Transpose +12: 880 Hz, and the clip
// keeps its length. Repitch, for contrast: 440 × 80/120 ≈ 293 Hz.
//
// Screenshot of the warp editor (Complex selected, no "plays as Repitch" note) for the PR:
// `WARP_SHOTS=<dir> npx playwright test warp-complex`.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { openClip } from "./clips";
import { addDevice, launch, newProject, openDeviceTab, pickOption, playButton, setNumberField, stopButton, openDeviceWindow, moveDeviceWindow } from "./ui";

const shots = process.env.WARP_SHOTS;

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const audioOf = async (page: Page, clip: string) => {
  const c: Clip = (await doc(page)).clips[clip]!;
  if (c.content.type !== "Audio") throw new Error("not audio");
  return c.content;
};

/** A mono 16-bit WAV: `seconds` of a `hz` sine at 48 kHz. */
function sineWav(seconds: number, hz: number): Buffer {
  const rate = 48000;
  const frames = Math.round(rate * seconds);
  const b = Buffer.alloc(44 + frames * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + frames * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(12000 * Math.sin((2 * Math.PI * hz * i) / rate)), 44 + i * 2);
  return b;
}

test("Complex warp keeps pitch at another tempo and transposes without changing length", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.setViewportSize({ width: 1440, height: 900 });
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Warp complex ${Date.now()}`);

  // --- An 8 s 440 Hz sine on a new audio track (imported through the file picker) -----
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (await chooser).setFiles([{ name: "Sine 440.wav", mimeType: "audio/wav", buffer: sineWav(8, 440) }]);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  const track = (await doc(page)).tracks[clip.track]!;

  // --- A Tuner after it ---------------------------------------------------------------------
  await openDeviceTab(page, track.name);
  await addDevice(page, "Tuner");
  await expect.poll(async () => Object.values((await doc(page)).devices).filter((d) => d.track === track.id).length).toBe(1);
  const tuner = Object.values((await doc(page)).devices).find((d) => d.track === track.id)!;
  const readout = (await openDeviceWindow(page, tuner.id)).getByTestId("widget-tuner").locator(".eth-tuner__readout");
  // Its window out of the way of the clip and the editor drawer (top right).
  await moveDeviceWindow(page, tuner.id, { x: 1440 - 560 - 12, y: 60 });
  const hz = async () => {
    const m = /([\d.]+) Hz/.exec((await readout.textContent()) ?? "");
    return m ? Number(m[1]) : 0;
  };

  // --- Warp on (16 beats at 120 BPM), Complex, project at 80 BPM --------------------------
  await openClip(page, clip.id);
  const editor = page.getByTestId("warp-editor");
  await expect(editor).toBeVisible();
  await editor.getByLabel("warp", { exact: true }).click();
  await expect.poll(async () => (await audioOf(page, clip.id)).warp.enabled).toBe(true);
  await expect.poll(async () => (await audioOf(page, clip.id)).warp.source_bpm).toBe(120);
  await pickOption(editor, "warp mode", "complex");
  await expect.poll(async () => (await audioOf(page, clip.id)).warp.mode).toBe("Complex");
  // The browser stretches: no "plays as Repitch" note.
  await expect(page.getByTestId("warp-web-fallback")).toHaveCount(0);
  const length = (await doc(page)).clips[clip.id]!.length;

  const tempo = page.getByRole("toolbar", { name: "transport" }).getByLabel("tempo", { exact: true });
  await tempo.click();
  await setNumberField(tempo, 80);
  await expect.poll(async () => Object.values((await doc(page)).tempo_points)[0]?.bpm).toBe(80);

  if (shots) {
    await page.mouse.move(0, 0);
    await editor.screenshot({ path: `${shots}/warp-editor-complex-web.png` });
  }

  /** Play from the start and wait for the tuner to settle within 1% of `want`. */
  const expectPitch = async (want: number) => {
    await playButton(page).click();
    await expect.poll(async () => Math.abs((await hz()) - want) / want, { timeout: 20_000 }).toBeLessThan(0.01);
    await stopButton(page).click();
    await expect(readout).toHaveText("no pitch", { timeout: 10_000 });
  };

  // Complex at 80 BPM: the 440 Hz sine stays at 440 Hz.
  await expectPitch(440);

  // Transpose +12 (Complex): 880 Hz, same clip length.
  await setNumberField(editor.getByRole("spinbutton", { name: "transpose" }), 12);
  await expect.poll(async () => (await audioOf(page, clip.id)).transpose).toBe(12);
  await expectPitch(880);
  expect((await doc(page)).clips[clip.id]!.length).toBe(length);

  // Repitch for contrast (transpose back to 0): the slower tempo lowers the pitch.
  await setNumberField(editor.getByRole("spinbutton", { name: "transpose" }), 0);
  await expect.poll(async () => (await audioOf(page, clip.id)).transpose).toBe(0);
  await pickOption(editor, "warp mode", "repitch");
  await expect.poll(async () => (await audioOf(page, clip.id)).warp.mode).toBe("Repitch");
  await expectPitch((440 * 80) / 120);

  expect(errors).toEqual([]);
});
