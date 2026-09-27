// Full user flow on the web build, through the UI, against the real engine: WasmTransport →
// controller Worker (EtherController) → AudioWorklet (ether-core), OPFS project store.
//
// create a project → MIDI track with the built-in synth → draw notes in the piano roll →
// audio track → drop a library sample on it (waveform from engine peaks) → compressor and
// delay → automate volume → play with loop on (playhead wraps, meters move) → edit, undo,
// redo → save → reload → identical state.
//
// No sleeps: every step waits on UI or engine state with `expect.poll` / auto-waiting
// locators. The UI mirror is read through `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip, selectClip } from "./clips";
import { addDevice, createTrack, newProject, openDeviceTab, openLibrary, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null; dirty: boolean; history: { can_undo: boolean; can_redo: boolean } };
  meter(track: string): { peak: [number, number] } | null;
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const count = (o: object) => Object.keys(o).length;

// At the default 1280 px width the transport bar's right side (undo/redo, CPU, engine
// status) overflows onto the Loop/Metronome buttons and takes their clicks (reported as an
// app layout bug); a wider window keeps them clickable.
test.use({ viewport: { width: 1600, height: 900 } });

/** Peak level of a track's latest meter frame (0 when none yet). */
const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** Playhead position in seconds, as shown by the transport bar (`m:ss.mmm`). */
const positionSeconds = (position: Locator) =>
  position.evaluate((el) => {
    const text = el.textContent ?? "";
    const m = /^(\d+):(\d+)\.(\d+)$/.exec(text.trim());
    return m ? Number(m[1]) * 60 + Number(m[2]) + Number(m[3]) / 1000 : NaN;
  });

/** Number of drawn (non-transparent) pixels in a canvas. */
const inkedPixels = (canvas: Locator) =>
  canvas.evaluate((c: HTMLCanvasElement) => {
    const ctx = c.getContext("2d");
    if (!ctx || c.width === 0 || c.height === 0) return 0;
    const data = ctx.getImageData(0, 0, c.width, c.height).data;
    let n = 0;
    for (let i = 3; i < data.length; i += 4) if (data[i]! > 0) n++;
    return n;
  });

test("full flow: build a song, play it, edit, save, reload", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  // --- Create a project -----------------------------------------------------------------
  const name = `E2E Song ${Date.now()}`;
  await newProject(page, name);
  const projectId = (await doc(page)).id;
  const baseTracks = count((await doc(page)).tracks);

  // --- MIDI track with the built-in synth -------------------------------------------------
  const midi = await createTrack(page, "Midi");
  expect(count((await doc(page)).tracks)).toBe(baseTracks + 1);
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id);
  expect(synth?.kind).toEqual({ type: "Builtin", device: { type: "Synth" } });

  // A one-bar MIDI clip (double-click in the empty lane), opened in the piano roll.
  const midiLane = page.locator(`[data-lane="${midi.id}"]`);
  await midiLane.dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => count((await doc(page)).clips)).toBe(1);
  const midiClip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, midiClip.id);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();

  // Draw notes: double-click on the grid inside the clip (left of the "outside" shade).
  const gridBox = (await grid.boundingBox())!;
  const endBox = (await page.getByTestId("piano-roll-clip-end").boundingBox())!;
  const clipWidth = endBox.x - gridBox.x;
  for (const [fx, fy] of [
    [0.02, 0.4],
    [0.27, 0.45],
    [0.52, 0.5],
    [0.77, 0.45],
  ] as const) {
    await grid.dblclick({ position: { x: clipWidth * fx + 2, y: gridBox.height * fy } });
  }
  await expect(page.getByTestId("piano-roll-note")).toHaveCount(4);
  await expect.poll(async () => count((await doc(page)).notes)).toBe(4);

  // --- Audio track + a library sample dropped on it ---------------------------------------
  const audio = await createTrack(page, "Audio");
  expect(count((await doc(page)).tracks)).toBe(baseTracks + 2);

  // The library location lists the bundled demo samples.
  // The sample browser is a pane opened from the rail; pinned, it doesn't cover the lanes.
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const sample = page.getByRole("button", { name: "Bass Loop 120.wav", exact: true });
  await expect(sample).toBeVisible();
  const audioLane = page.locator(`[data-lane="${audio.id}"]`);
  await sample.dragTo(audioLane, { targetPosition: { x: 5, y: 20 } });
  await expect
    .poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 })
    .toBe(1);
  const audioClip = Object.values((await doc(page)).clips).find((c) => c.track === audio.id)!;
  expect(count((await doc(page)).media)).toBe(1);
  // Import + clip are one undo step.
  const audioClips = async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length;
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(audioClips).toBe(0);
  await expect.poll(async () => count((await doc(page)).media)).toBe(0);
  await page.getByRole("button", { name: "Redo" }).click();
  await expect.poll(audioClips).toBe(1);
  expect(count((await doc(page)).media)).toBe(1);
  // Waveform drawn from engine peaks.
  const waveform = page.locator(`[data-clip-id="${audioClip.id}"] canvas`);
  await expect.poll(() => inkedPixels(waveform), { timeout: 20_000 }).toBeGreaterThan(50);

  // --- Compressor and delay on the audio track ---------------------------------------------
  // Selecting the track opens the inspector with its devices.
  await openDeviceTab(page, audio.name);
  await addDevice(page, "Compressor");
  await expect.poll(async () => Object.values((await doc(page)).devices).filter((d) => d.track === audio.id).length).toBe(1);
  await addDevice(page, "Delay");
  await expect
    .poll(async () =>
      Object.values((await doc(page)).devices)
        .filter((d) => d.track === audio.id)
        .sort((a, b) => (a.order < b.order ? -1 : 1))
        .map((d) => (d.kind.type === "Builtin" ? d.kind.device.type : d.kind.type)),
    )
    .toEqual(["Compressor", "Delay"]);

  // --- Automate the MIDI track's volume -----------------------------------------------------
  await page.getByRole("button", { name: `Show automation of ${midi.name}` }).click();
  const volumeLane = page.getByRole("group", { name: "Volume automation" }).getByTestId("automation-lane-svg");
  await expect(volumeLane).toBeVisible();
  const laneBox = (await volumeLane.boundingBox())!;
  await volumeLane.dblclick({ position: { x: 10, y: laneBox.height * 0.2 } });
  await expect.poll(async () => count((await doc(page)).automation_points)).toBe(1);
  await volumeLane.dblclick({ position: { x: 150, y: laneBox.height * 0.6 } });
  await expect.poll(async () => count((await doc(page)).automation_points)).toBe(2);
  const lanes = Object.values((await doc(page)).automation_lanes);
  expect(lanes.map((l) => l.target)).toEqual([{ type: "TrackVolume", track: midi.id }]);

  // --- Play with loop on: the playhead wraps, meters and playhead move ----------------------
  const position = page.getByTestId("position-time");
  const transportBar = page.getByRole("toolbar", { name: "Transport" });
  await transportBar.getByRole("button", { name: "Loop" }).click();
  await expect.poll(async () => (await doc(page)).settings.loop_enabled).toBe(true);
  await transportBar.getByRole("button", { name: "Play", exact: true }).click();
  await expect(transportBar.getByRole("button", { name: "Stop" }).first()).toBeVisible();
  await expect.poll(() => peakOf(page, midi.id), { timeout: 10_000 }).toBeGreaterThan(0.001);
  await expect.poll(() => peakOf(page, audio.id), { timeout: 10_000 }).toBeGreaterThan(0.001);
  // The loop region is 0..16 beats (8 s at 120 BPM): the position passes 2 s, then wraps
  // back below it.
  await expect.poll(() => positionSeconds(position), { timeout: 15_000 }).toBeGreaterThan(2);
  await expect.poll(() => positionSeconds(position), { timeout: 15_000 }).toBeLessThan(2);
  await transportBar.getByRole("button", { name: "Stop" }).first().click();
  await expect(transportBar.getByRole("button", { name: "Play", exact: true })).toBeVisible();

  // --- Edit, undo, redo ----------------------------------------------------------------------
  await selectClip(page, midiClip.id);
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(async () => count((await doc(page)).clips)).toBe(3);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => count((await doc(page)).clips)).toBe(2);
  await page.getByRole("button", { name: "Redo" }).click();
  await expect.poll(async () => count((await doc(page)).clips)).toBe(3);
  await expect.poll(async () => count((await doc(page)).notes)).toBe(8);

  // --- Save, reload, identical state -------------------------------------------------------
  // The project autosaves a second after the last change; Ctrl/Cmd+S saves now.
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByRole("status", { name: "Unsaved changes" })).toHaveCount(0);
  const saved = await doc(page);

  await page.reload();
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p?.id), { timeout: 30_000 }).toBe(projectId);
  expect(await doc(page)).toEqual(saved);
  await expect(page.getByTestId("project-name")).toHaveText(name);
  // The reloaded audio clip draws its waveform again (peaks from the reopened media).
  await expect.poll(() => inkedPixels(page.locator(`[data-clip-id="${audioClip.id}"] canvas`)), { timeout: 20_000 }).toBeGreaterThan(50);

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
