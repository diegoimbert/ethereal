// Clip gain on the web build, through the UI, against the real engine: changing an audio
// clip's gain in the Inspector redraws its arrangement waveform from the cached peaks,
// shorter (-12 dB) or taller (+6 dB), flattened and marked where it passes full scale.
//
// No sleeps: every step waits on UI or engine state. The drawn waveform is compared by the
// sum of its canvas alpha (the ink covering the clip body).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { selectClip } from "./clips";
import { openLibrary, setNumberField, launch } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

/** Sum of the alpha channel over a clip's waveform canvas (0 until peaks are drawn). */
const inkOf = (page: Page, clipId: string): Promise<number> =>
  page.evaluate((id) => {
    const cv = document.querySelector<HTMLCanvasElement>(`[data-clip-id="${id}"] [data-testid="clip-waveform"]`);
    const ctx = cv?.getContext("2d");
    if (!cv || !ctx || cv.width === 0 || cv.height === 0) return 0;
    const data = ctx.getImageData(0, 0, cv.width, cv.height).data;
    let sum = 0;
    for (let i = 3; i < data.length; i += 4) sum += data[i]!;
    return sum;
  }, clipId);

test("clip gain: the Inspector gain redraws the arrangement waveform", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Clip gain ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);

  // --- Audio track + library loop ----------------------------------------------------------
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const sample = page.getByRole("button", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  const wave = page.locator(`[data-clip-id="${clip.id}"] [data-testid="clip-waveform"]`);

  // --- Baseline at 0 dB, once the peaks have arrived ------------------------------------------
  await selectClip(page, clip.id);
  const gain = page.getByTestId("inspector").getByRole("spinbutton", { name: "Clip gain" });
  await expect(gain).toBeVisible();
  await expect(wave).toHaveAttribute("data-gain", "0");
  let base = 0;
  await expect
    .poll(
      async () => {
        const ink = await inkOf(page, clip.id);
        const stable = ink > 0 && ink === base;
        base = ink;
        return stable;
      },
      { timeout: 20_000 },
    )
    .toBe(true);

  // --- -12 dB: a shorter waveform -------------------------------------------------------------
  await setNumberField(gain, -12);
  await expect(wave).toHaveAttribute("data-gain", "-12");
  await expect.poll(() => inkOf(page, clip.id)).toBeLessThan(base * 0.6);

  // --- +6 dB: a taller waveform ---------------------------------------------------------------
  await setNumberField(gain, 6);
  await expect(wave).toHaveAttribute("data-gain", "6");
  await expect.poll(() => inkOf(page, clip.id)).toBeGreaterThan(base * 1.2);

  // --- +24 dB: flattened at the lane edge and marked where it passes full scale --------------
  await expect(wave).toHaveAttribute("data-clipped", "0");
  await setNumberField(gain, 24);
  await expect(wave).toHaveAttribute("data-gain", "24");
  await expect.poll(async () => Number(await wave.getAttribute("data-clipped"))).toBeGreaterThan(0);

  expect(errors).toEqual([]);
});
