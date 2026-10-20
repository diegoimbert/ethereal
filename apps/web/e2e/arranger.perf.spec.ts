// Arranger frame rate (perf): 64 tracks × 8 MIDI clips on the web build. Playback and wheel
// scrolling keep a steady frame rate, and the arranger doesn't promote a compositor layer
// per clip (one canvas per clip made ~650 layers and ~20 fps).
//
// - Structural, always asserted: the page's compositor layer count stays far below one per
//   clip.
// - Timing, asserted when the host has CPU headroom (`hostHasHeadroom`): frame intervals
//   while playing and scrolling against the same page at rest.
//
// `ETHER_E2E_TRACE=<dir>` also records a Chromium trace of each phase there and logs the
// time spent in Layerize / Paint per frame (the audit's method).
import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test, type Browser, type Page } from "@playwright/test";
import { arrangerApi, clipPart } from "./clips";
import { launch, newProject, playButton } from "./ui";
import { buildPerfProject, hostHasHeadroom, idleFrames, layerCount, median, metrics, p95, startFrames, stopFrames } from "./perfProject";

test.use({ viewport: { width: 1440, height: 900 }, trace: "off" });

/**
 * Layers allowed for the whole page: the old arranger had about one per clip (512 clips).
 * Each track header and its meter are layers of their own on purpose (they no
 * longer re-layerize against the lanes on every pan/zoom frame): about 2 per track (64
 * here) plus the lane canvases, measured at 249.
 */
const MAX_LAYERS = 300;

const traceDir = process.env.ETHER_E2E_TRACE;

async function traced(browser: Browser, page: Page, name: string, run: () => Promise<number[]>): Promise<number[]> {
  if (!traceDir) return run();
  mkdirSync(traceDir, { recursive: true });
  const path = join(traceDir, `${name}.json`);
  await browser.startTracing(page, {
    path,
    categories: [
      "devtools.timeline",
      "disabled-by-default-devtools.timeline",
      "disabled-by-default-devtools.timeline.frame",
      "disabled-by-default-devtools.timeline.invalidationTracking",
    ],
  });
  const out = await run();
  await browser.stopTracing();
  const events = (JSON.parse(readFileSync(path, "utf8")) as { traceEvents: { name: string; ph: string; dur?: number }[] }).traceEvents;
  const sum = (n: string) => events.filter((e) => e.name === n && e.ph === "X").reduce((a, e) => a + (e.dur ?? 0), 0) / 1000;
  const frames = Math.max(1, out.length);
  console.log(
    `arranger perf trace ${name}: per frame: Layerize ${(sum("Layerize") / frames).toFixed(2)} ms, ` +
      `Paint ${(sum("Paint") / frames).toFixed(2)} ms, Layout ${(sum("Layout") / frames).toFixed(2)} ms, ` +
      `UpdateLayoutTree ${(sum("UpdateLayoutTree") / frames).toFixed(2)} ms, FunctionCall ${(sum("FunctionCall") / frames).toFixed(2)} ms`,
  );
  return out;
}

test("64 tracks × 8 clips: playback and scrolling keep the frame rate @frames", async ({ page, browser }) => {
  test.setTimeout(240_000);
  await launch(page);
  await newProject(page, `Arranger perf ${Date.now()}`);
  await buildPerfProject(page);
  await expect.poll(async () => (await arrangerApi(page, "visibleClipIds")).length).toBeGreaterThan(0);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");

  const rest = await idleFrames(page);
  const layers = await layerCount(cdp, page);

  const phase = async (name: string, act: () => Promise<void>) => {
    const m0 = await metrics(cdp);
    const frames = await traced(browser, page, name, async () => {
      await startFrames(page);
      await act();
      return stopFrames(page);
    });
    const m1 = await metrics(cdp);
    const per = (k: "script" | "layout" | "task") => ((m1[k] - m0[k]) * 1000) / frames.length;
    console.log(
      `arranger perf ${name}: ${frames.length} frames, median ${median(frames).toFixed(1)} ms, p95 ${p95(frames).toFixed(1)} ms ` +
        `(rest ${median(rest).toFixed(1)} ms); per frame: script ${per("script").toFixed(2)} ms, style+layout ${per("layout").toFixed(2)} ms, ` +
        `main-thread tasks ${per("task").toFixed(2)} ms`,
    );
    return frames;
  };

  // Playback: the playhead moves over the lanes for ~2 s.
  const play = await phase("playback", async () => {
    await playButton(page).click();
    await page.waitForTimeout(2_000);
  });
  await page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "stop", exact: true }).first().click();
  await expect(playButton(page)).toBeVisible();

  // Wheel: vertical scrolling through the tracks, then horizontal scrolling of the timeline.
  const lanes = page.locator(".eth-arr__scroll");
  const box = (await lanes.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const wheel = await phase("wheel", async () => {
    for (let i = 0; i < 30; i++) {
      await page.mouse.wheel(0, i < 15 ? 120 : -120);
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
    }
    for (let i = 0; i < 30; i++) {
      await page.mouse.wheel(i < 15 ? 120 : -120, 0);
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
    }
  });

  // Clip drag: every pointermove previews the move (it re-rendered every lane).
  const first = (await arrangerApi(page, "visibleClipIds"))[0]!;
  const t = await clipPart(page, first, "header");
  await page.mouse.move(t.x + 6, t.y + 4);
  await page.mouse.down();
  const drag = await phase("drag", async () => {
    for (let i = 1; i <= 40; i++) await page.mouse.move(t.x + 6 + i * 4, t.y + 4 + (i % 2) * 2);
  });
  await page.mouse.up();

  console.log(`arranger perf: ${layers} compositor layers at rest`);
  expect.soft(layers).toBeLessThanOrEqual(MAX_LAYERS);

  // The owner's limit: frame timing is asserted up to a load of 16 (half this devbox's cores).
  const host = hostHasHeadroom(16);
  console.log(`arranger perf: ${host.why}: timing ${host.ok ? "asserted" : "not asserted (host oversubscribed)"}`);
  if (!host.ok) {
    test.info().annotations.push({ type: "perf", description: `timing not asserted: host oversubscribed (${host.why})` });
    return;
  }
  // Two frames' budget at the resting rate for the slowest 5% (the audit measured 4 frames).
  expect(p95(play)).toBeLessThanOrEqual(median(rest) * 2.5);
  expect(median(play)).toBeLessThanOrEqual(median(rest) * 1.25);
  expect(p95(wheel)).toBeLessThanOrEqual(median(rest) * 2.5);
  expect(p95(drag)).toBeLessThanOrEqual(median(rest) * 2.5);
});
