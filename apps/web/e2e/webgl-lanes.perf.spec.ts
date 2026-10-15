// WebGL lane layers perf (PR numbers): an automation lane of 10,000 points (linear, step and
// curve segments), panned (one view change per animation frame, like the wheel's motion) at
// a zoom where thousands of points are on screen, on both backends (WebGL, and the Canvas2D
// fallback for reference):
// - frame intervals (median, p95) and main-thread script time per frame;
// - the renderer's own draw time per frame (`stats.frameMs`).
//
// Timing is logged, and asserted for WebGL only when the host has CPU headroom
// (`hostHasHeadroom`): the main thread fits a 60 fps frame with room to spare (p95 draw time
// under 4 ms, script under 4 ms per frame). Headless Chromium renders WebGL with SwiftShader
// (on the CPU): its frame intervals measure the software rasterizer, not a GPU (as in
// webgl-arranger.perf.spec.ts).
import { expect, test, type Page } from "@playwright/test";
import type { Command, PointSpec } from "@/generated";
import { arrangerApi, setArrangerRenderer } from "./clips";
import { hostHasHeadroom, installProbe, median, metrics, p95, sendBatched, setViewport, ulid } from "./perfProject";
import { headerButton, launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, trace: "off" });

const POINTS = 10_000;

interface Stats {
  frameMs: number[];
  drawCalls: number;
  instances: number;
}

function pan(page: Page, frames: number): Promise<number[]> {
  return page.evaluate(
    (frames) =>
      new Promise<number[]>((done) => {
        const view = (
          window as unknown as { __ether: { arrangementView(): { getState(): { pxPerBeat: number; scrollBeats: number; setViewport(v: object): void } } } }
        ).__ether.arrangementView();
        const out: number[] = [];
        let last = performance.now();
        let i = 0;
        const tick = (now: number) => {
          out.push(now - last);
          last = now;
          const s = view.getState();
          s.setViewport({ scrollBeats: s.scrollBeats + (i < frames / 2 ? 12 : -12) / s.pxPerBeat });
          if (++i < frames) requestAnimationFrame(tick);
          else done(out.slice(1));
        };
        requestAnimationFrame(tick);
      }),
    frames,
  );
}

test(`automation lane of ${POINTS} points: pan frame times @frames`, async ({ page }) => {
  test.setTimeout(300_000);
  await launch(page, `Lanes perf ${Date.now()}`);
  await installProbe(page);
  const track = ulid(8, 0);
  const lane = ulid(0, 0);
  const curves: PointSpec["curve"][] = [{ type: "Linear" }, { type: "Step" }, { type: "Curve", tension: 0.5 }];
  const points: PointSpec[] = Array.from({ length: POINTS }, (_, i) => ({
    id: ulid(2, i),
    time: i * 0.25,
    value: 0.5 + 0.45 * Math.sin(i / 7),
    curve: curves[i % 3]!,
  }));
  const cmds: Command[] = [
    { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name: "Automated", color: null, parent: null, before: null } },
    { domain: "Automation", command: { type: "CreateLane", id: lane, owner: { type: "Track", track }, target: { type: "TrackVolume", track } } },
  ];
  for (let i = 0; i < POINTS; i += 1000) cmds.push({ domain: "Automation", command: { type: "AddPoints", lane, points: points.slice(i, i + 1000) } });
  await sendBatched(page, cmds, 4);
  await (await headerButton(page, "Automated", "show automation of Automated")).click();
  await expect(page.getByRole("group", { name: "volume automation" })).toBeVisible();
  const close = page.getByRole("button", { name: /^close / });
  while ((await close.count()) > 0) await close.first().click();

  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  let webglDraws: number[] = [];
  let webglScript = 0;
  // The Canvas2D fallback for reference (logged), then WebGL (asserted).
  for (const renderer of ["canvas2d", "webgl"] as const) {
    await setArrangerRenderer(page, renderer);
    // 8 px per beat: ~150 beats, ~600 points on screen; 1 px per beat: every point in 2500 beats.
    for (const ppb of [8, 1]) {
      await setViewport(page, ppb, 100);
      await page.waitForTimeout(1000);
      const before = ((await arrangerApi(page, "stats")) as Stats | null)?.frameMs.length ?? 0;
      const m0 = await metrics(cdp);
      const frames = await pan(page, 120);
      const m1 = await metrics(cdp);
      const stats = (await arrangerApi(page, "stats")) as Stats | null;
      const draws = stats ? stats.frameMs.slice(before) : [];
      const script = ((m1.script - m0.script) * 1000) / frames.length;
      const line =
        `${renderer} ${POINTS} points at ${ppb} px/beat pan: frames median ${median(frames).toFixed(1)} ms, p95 ${p95(frames).toFixed(1)} ms; ` +
        `script ${script.toFixed(2)} ms/frame` +
        (draws.length ? `; draw median ${median(draws).toFixed(2)} ms, p95 ${p95(draws).toFixed(2)} ms, ${stats!.instances} instances` : "");
      console.log(line);
      test.info().annotations.push({ type: "perf", description: line });
      if (renderer === "webgl") {
        webglDraws = [...webglDraws, ...draws];
        webglScript = Math.max(webglScript, script);
      }
    }
  }
  expect(webglDraws.length).toBeGreaterThan(0);
  const host = hostHasHeadroom();
  console.log(`lanes perf: ${host.why}: timing ${host.ok ? "asserted" : "not asserted (host oversubscribed)"}`);
  if (host.ok) {
    expect(webglScript).toBeLessThan(4);
    expect(p95(webglDraws)).toBeLessThan(4);
  }
});
