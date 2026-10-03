// Warp editor on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// audio track + library loop (unwarped) → double-click the clip: Warp tab → warp on: the
// controller's BPM stub pins two markers → Complex shows the browser fallback note (no
// stretcher in wasm: Complex plays as Repitch) → warp off/on keeps them → add a marker
// (double-click) → drag it (one undo step) → transpose → play: signal on the track meter.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Clip, Project, WarpMarker } from "@/generated";
import { openClip } from "./clips";
import { createTrack, launch, newProject, openLibrary, pickOption, playButton, setNumberField } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const markersOf = async (page: Page, clip: string): Promise<WarpMarker[]> =>
  Object.values((await doc(page)).warp_markers)
    .filter((m) => m.clip === clip)
    .sort((a, b) => a.beat - b.beat);

const audioOf = async (page: Page, clip: string) => {
  const c: Clip = (await doc(page)).clips[clip]!;
  if (c.content.type !== "Audio") throw new Error("not audio");
  return c.content;
};

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

const inkedPixels = (canvas: Locator) =>
  canvas.evaluate((c: HTMLCanvasElement) => {
    const ctx = c.getContext("2d");
    if (!ctx || c.width === 0 || c.height === 0) return 0;
    const data = ctx.getImageData(0, 0, c.width, c.height).data;
    let n = 0;
    for (let i = 3; i < data.length; i += 4) if (data[i]! > 0) n++;
    return n;
  });

test("warp: markers, modes, transpose and playback", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await newProject(page, `Warp ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);

  // --- Audio track + library loop ----------------------------------------------------------
  const audio = await createTrack(page, "Audio");
  // The sample browser is a pane opened from the rail; pinned, it doesn't cover the lanes.
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const sample = page.getByRole("button", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;

  // --- Double-click opens the Warp tab -----------------------------------------------------
  await openClip(page, clip.id);
  const editor = page.getByTestId("warp-editor");
  await expect(editor).toBeVisible();
  await expect(editor).toHaveAttribute("data-clip-id", clip.id);
  await expect.poll(() => inkedPixels(page.getByTestId("warp-waveform")), { timeout: 20_000 }).toBeGreaterThan(50);

  // --- New clips are unwarped; enabling warp lets the BPM stub pin start and end ---------
  // The checkbox is controlled by the document: click, then wait for the patch.
  const warpBox = editor.getByLabel("Warp", { exact: true });
  await expect(warpBox).not.toBeChecked();
  expect((await audioOf(page, clip.id)).warp.enabled).toBe(false);
  await warpBox.click();
  await expect(warpBox).toBeChecked();
  await expect.poll(async () => (await markersOf(page, clip.id)).length).toBe(2);
  await expect(page.getByTestId("warp-marker")).toHaveCount(2);

  // Complex has no stretcher in the browser: the editor says it plays as Repitch. (The
  // inspector has a "Warp mode" picker too: use the editor's.)
  await pickOption(editor, "Warp mode", "Complex");
  await expect(page.getByTestId("warp-web-fallback")).toBeVisible();
  await expect.poll(async () => (await audioOf(page, clip.id)).warp.mode).toBe("Complex");

  // Off/on keeps the markers.
  await warpBox.click();
  await expect(warpBox).not.toBeChecked();
  await warpBox.click();
  await expect(warpBox).toBeChecked();
  expect(await markersOf(page, clip.id)).toHaveLength(2);
  const [first] = await markersOf(page, clip.id);
  expect(first!.beat).toBe(0);
  expect(first!.source).toBe(0);

  // --- Add a marker, then drag it (one undo step) ------------------------------------------
  const view = page.getByTestId("warp-view");
  const box = (await view.boundingBox())!;
  const pxPerBeat = Number(await view.getAttribute("data-px-per-beat"));
  await view.dblclick({ position: { x: 2 * pxPerBeat, y: box.height / 2 } });
  await expect.poll(async () => (await markersOf(page, clip.id)).length).toBe(3);
  const added = (await markersOf(page, clip.id)).find((m) => Math.abs(m.beat - 2) < 1e-9)!;
  const handle = page.locator(`[data-testid="warp-marker"][data-marker-id="${added.id}"] path`);
  const hb = (await handle.boundingBox())!;
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  for (const dx of [0.25, 0.5, 1]) await page.mouse.move(hb.x + hb.width / 2 + dx * pxPerBeat, hb.y + hb.height / 2);
  await page.mouse.up();
  await expect.poll(async () => (await doc(page)).warp_markers[added.id]?.beat).toBe(3);
  expect((await doc(page)).warp_markers[added.id]!.source).toBe(added.source);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await doc(page)).warp_markers[added.id]?.beat).toBe(2);

  // --- Transpose + play: the clip sounds -----------------------------------------------------
  await setNumberField(editor.getByRole("spinbutton", { name: "Transpose" }), 3);
  await expect.poll(async () => (await audioOf(page, clip.id)).transpose).toBe(3);
  await playButton(page).click();
  await expect.poll(() => peakOf(page, audio.id), { timeout: 20_000 }).toBeGreaterThan(0.01);
  await page.getByRole("button", { name: "Stop", description: "Stop (Space)" }).click();

  expect(errors).toEqual([]);
});
