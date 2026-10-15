// WebGL arranger perf (PR numbers): 1k and 5k clips over 200 tracks (audio with waveforms
// and MIDI with note previews), a programmatic pan and zoom (one view change per animation
// frame, like the wheel's motion), measured on both backends (WebGL, and the Canvas2D
// fallback for reference):
// - frame intervals (median, p95);
// - main-thread script time per frame (CDP `ScriptDuration`), and the renderer's own draw
//   time per frame (`stats.frameMs`);
// - GPU memory of the WebGL renderer (instance buffers, peak tiles, glyph atlas).
//
// Timing is logged, and asserted only for WebGL when the host has CPU headroom
// (`hostHasHeadroom`): p95 draw time under 4 ms. Headless Chromium renders WebGL with
// SwiftShader (on the CPU): frame intervals there say little about a real GPU, the
// main-thread costs do.
import { expect, test, type CDPSession, type Page } from "@playwright/test";
import { arrangerApi, setArrangerRenderer } from "./clips";
import { buildMixedProject, hostHasHeadroom, importBassLoop, median, metrics, p95, setViewport } from "./perfProject";
import { launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, trace: "off" });

interface Stats {
  frameMs: number[];
  gpuBytes: { instances: number; waveTiles: number; atlas: number };
  drawCalls: number;
  instances: number;
}

/** Runs `frames` animation frames, each changing the view; frame intervals (ms). */
function animate(page: Page, kind: "pan" | "zoom", frames: number): Promise<number[]> {
  return page.evaluate(
    ([kind, frames]) =>
      new Promise<number[]>((done) => {
        const view = (
          window as unknown as {
            __ether: {
              arrangementView(): {
                getState(): { pxPerBeat: number; scrollBeats: number; widthPx: number; setViewport(v: object): void; zoomBy(f: number, a?: number): void };
              };
            };
          }
        ).__ether.arrangementView();
        const out: number[] = [];
        let last = performance.now();
        let i = 0;
        const tick = (now: number) => {
          out.push(now - last);
          last = now;
          const s = view.getState();
          if (kind === "pan") s.setViewport({ scrollBeats: s.scrollBeats + (i < frames / 2 ? 12 : -12) / s.pxPerBeat });
          else s.zoomBy(i < frames / 2 ? 0.96 : 1 / 0.96, s.widthPx / 2);
          if (++i < frames) requestAnimationFrame(tick);
          else done(out.slice(1));
        };
        requestAnimationFrame(tick);
      }),
    [kind, frames] as const,
  );
}

async function measure(page: Page, cdp: CDPSession, label: string, kind: "pan" | "zoom", start: { ppb: number; scroll: number }) {
  await setViewport(page, start.ppb, start.scroll);
  await page.waitForTimeout(1500); // peaks and settled redraws
  const before = ((await arrangerApi(page, "stats")) as Stats | null)?.frameMs.length ?? 0;
  const m0 = await metrics(cdp);
  const frames = await animate(page, kind, 120);
  const m1 = await metrics(cdp);
  const stats = (await arrangerApi(page, "stats")) as Stats | null;
  const draws = stats ? stats.frameMs.slice(Math.max(0, stats.frameMs.length - (stats.frameMs.length - before))) : [];
  const script = ((m1.script - m0.script) * 1000) / frames.length;
  const task = ((m1.task - m0.task) * 1000) / frames.length;
  const layout = ((m1.layout - m0.layout) * 1000) / frames.length;
  const line =
    `${label} ${kind}: frames median ${median(frames).toFixed(1)} ms, p95 ${p95(frames).toFixed(1)} ms; ` +
    `main thread per frame: script ${script.toFixed(2)} ms, style+layout ${layout.toFixed(2)} ms, tasks ${task.toFixed(2)} ms` +
    (draws.length ? `; draw median ${median(draws).toFixed(2)} ms, p95 ${p95(draws).toFixed(2)} ms (${draws.length} draws)` : "");
  console.log(line);
  test.info().annotations.push({ type: "perf", description: line });
  return { frames, draws, script };
}

for (const clips of [1000, 5000]) {
  test(`arranger renderers, ${clips} clips over 200 tracks: pan and zoom frame times @frames`, async ({ page }) => {
    test.setTimeout(900_000);
    await launch(page, `Arranger perf ${clips} ${Date.now()}`);
    await buildMixedProject(page, { tracks: 200, clips, importSample: (t) => importBassLoop(page, t) });
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Performance.enable");
    const results: Record<string, { draws: number[] }> = {};
    // WebGL last: the memory numbers below are its.
    for (const renderer of ["canvas2d", "webgl"] as const) {
      await setArrangerRenderer(page, renderer);
      const label = `${renderer} ${clips} clips`;
      await measure(page, cdp, label, "pan", { ppb: 24, scroll: 0 });
      // Zoomed out: every clip of the visible tracks on screen.
      const out = await measure(page, cdp, `${label} (zoomed out)`, "pan", { ppb: 0.5, scroll: 0 });
      const zoom = await measure(page, cdp, label, "zoom", { ppb: 8, scroll: 0 });
      results[renderer] = { draws: [...out.draws, ...zoom.draws] };
    }
    await setViewport(page, 24, 0);
    await page.waitForTimeout(500);
    const stats = (await arrangerApi(page, "stats")) as Stats;
    const mb = (b: number) => (b / 1048576).toFixed(1);
    const mem = `WebGL GPU memory: instances ${mb(stats.gpuBytes.instances)} MB, peak tiles ${mb(stats.gpuBytes.waveTiles)} MB, glyph atlas ${mb(stats.gpuBytes.atlas)} MB; a frame at rest (normal zoom): ${stats.drawCalls} draw calls, ${stats.instances} instances`;
    console.log(mem);
    test.info().annotations.push({ type: "perf", description: mem });

    const host = hostHasHeadroom();
    console.log(`arranger perf: ${host.why}: timing ${host.ok ? "asserted" : "not asserted (host oversubscribed)"}`);
    if (host.ok) expect(p95(results.webgl!.draws)).toBeLessThan(4);
  });
}
