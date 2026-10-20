// Mobile performance and memory (the design's §9 and §11), on the Pixel 7 profile with the
// CPU throttled 4× (a mid-range phone), a 16-track project:
// - a 2 s one-finger pan and a 2 s pinch through real touch input (CDP): frame intervals
//   (median, p95) and long tasks during the gesture. The budgets (p95 ≤ 20 ms on WebGL, no
//   long task over 50 ms) are for a real GPU; headless Chromium renders WebGL on the CPU
//   (SwiftShader), so here the numbers are logged and the assertion is relative: the
//   throttled phone run against the same gesture unthrottled, which catches per-frame work that
//   grows with the CPU cost (React renders, layout) rather than the GPU's;
// - the timeline canvases' pixel ratio is capped at 2 (a DPR 3 iPhone fills 2.25× fewer pixels);
// - memory: the tab (its renderer processes: page, workers, worklet, wasm) stays under budget
//   with the project open, after a garbage collection and before it, and the page is
//   cross-origin isolated (SharedArrayBuffer).
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { buildPerfProject, hostHasHeadroom, median, p95, startFrames, stopFrames } from "./perfProject";
import { arrangementViewState, IPHONE, launchMobile, PIXEL, touchscreen } from "./mobile";

test.use({ trace: "off" });

/**
 * The tab with 20 minutes of mono audio open. Measured 555 MB on 2026-10-13 (the
 * 16-track MIDI project: 338 MB): the decoded samples (32-bit float, 2× the 16-bit files)
 * dominate. The budget leaves about a quarter of headroom.
 */
const BUDGET_LONG_AUDIO_MB = 700;
/**
 * The tab with the 16-track MIDI project, after a garbage collection of the page: the memory it
 * holds. 280 MB: 236-241 MB measured (median 239, 7 runs) on 2026-10-20 (#136; 242 MB at
 * 92a33e13, 250 MB before #136's fixes). Single runs vary by a few MB. A build with more of its
 * own UI sets its budget with `ETHER_E2E_MOBILE_MEMORY_MB`.
 */
const BUDGET_MB = Number(process.env.ETHER_E2E_MOBILE_MEMORY_MB ?? 280);
/**
 * The same tab before that collection: what it holds plus the garbage the gestures left, which
 * depends on when the collector last ran (a loaded host runs it later, so this varies by ±30 MB
 * and more on CI). 400 MB: 287-329 MB measured (median 293) on 2026-10-20. A build with more of
 * its own UI sets it with `ETHER_E2E_MOBILE_MEMORY_UNCOLLECTED_MB`.
 */
const BUDGET_UNCOLLECTED_MB = Number(process.env.ETHER_E2E_MOBILE_MEMORY_UNCOLLECTED_MB ?? 400);

/** The lanes' box (right of the header column). */
async function lanes(page: Page) {
  const box = (await page.locator(".eth-arr__scroll").boundingBox())!;
  const headerWidth = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector(".eth-arr")!).getPropertyValue("--eth-arr-header-width")));
  return { x: box.x + headerWidth, y: box.y, width: box.width - headerWidth, height: box.height };
}

/**
 * The tab's memory: the resident anonymous and shared memory of the browser's renderer
 * processes (the page, its workers, the audio worklet and the engine's wasm memory all live
 * there; one page is open). `performance.measureUserAgentSpecificMemory` is not available in
 * headless Chromium, so the PIDs come from CDP (`SystemInfo.getProcessInfo`) and the sizes from
 * /proc (Linux); null elsewhere.
 */
async function tabMemory(page: Page): Promise<number | null> {
  const browser = page.context().browser();
  if (!browser || !existsSync("/proc/self/status")) return null;
  const cdp = await browser.newBrowserCDPSession();
  const { processInfo } = (await cdp.send("SystemInfo.getProcessInfo")) as { processInfo: Array<{ type: string; id: number }> };
  await cdp.detach();
  let total = 0;
  for (const p of processInfo.filter((p) => p.type === "renderer")) {
    const status = readFileSync(`/proc/${p.id}/status`, "utf8");
    const kb = (key: string) => Number(new RegExp(`^${key}:\\s+(\\d+) kB`, "m").exec(status)?.[1] ?? 0);
    total += (kb("RssAnon") + kb("RssShmem")) * 1024;
  }
  return total;
}

/**
 * The memory tests' page, in a browser of its own: the worker's browser may still hold the
 * renderer process of a test that ran before (about 20 MB more), which `tabMemory` would count.
 */
const memoryTest = test.extend<{ freshPage: Page }>({
  freshPage: async ({ playwright, launchOptions, baseURL, storageState, viewport, userAgent, deviceScaleFactor, isMobile, hasTouch }, provide) => {
    const browser = await playwright.chromium.launch(launchOptions);
    const context = await browser.newContext({ baseURL, storageState, viewport, userAgent, deviceScaleFactor, isMobile, hasTouch });
    await provide(await context.newPage());
    await browser.close();
  },
});

/** Long tasks (ms) from now on; `takeLongTasks` returns and clears them. */
const watchLongTasks = (page: Page) =>
  page.evaluate(() => {
    const w = window as unknown as { __long: number[]; __longWatched?: boolean };
    w.__long = [];
    if (w.__longWatched) return;
    w.__longWatched = true;
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) w.__long.push(e.duration);
    }).observe({ type: "longtask" });
  });
const takeLongTasks = (page: Page) =>
  page.evaluate(() => {
    const w = window as unknown as { __long: number[] };
    const out = w.__long;
    w.__long = [];
    return out;
  });

/** A 2 s pan (sideways, then back) and a 2 s pinch (out, then in): frame intervals and long tasks of each. */
async function gestures(page: Page, label: string) {
  const touch = await touchscreen(page);
  const l = await lanes(page);
  const mid = { x: l.x + l.width / 2, y: l.y + Math.min(l.height / 2, 200) };
  const out = {} as Record<"pan" | "pinch", { frames: number[]; long: number[] }>;

  await watchLongTasks(page);
  const before = await arrangementViewState(page);
  await startFrames(page);
  await touch.drag({ x: mid.x + l.width / 3, y: mid.y }, { x: mid.x - l.width / 3, y: mid.y + 4 }, { steps: 60, ms: 1000, rest: 0 });
  await touch.drag({ x: mid.x - l.width / 3, y: mid.y }, { x: mid.x + l.width / 3, y: mid.y + 4 }, { steps: 60, ms: 1000, rest: 200 });
  out.pan = { frames: await stopFrames(page), long: await takeLongTasks(page) };
  const panned = await arrangementViewState(page);
  expect(panned.scrollBeats, "the pan moved the view").not.toBe(before.scrollBeats);

  await startFrames(page);
  const d = l.width / 3;
  await touch.pinch({ x: mid.x - 20, y: mid.y }, { x: mid.x + 20, y: mid.y }, { x: mid.x - d, y: mid.y }, { x: mid.x + d, y: mid.y }, 60);
  await touch.pinch({ x: mid.x - d, y: mid.y }, { x: mid.x + d, y: mid.y }, { x: mid.x - 30, y: mid.y }, { x: mid.x + 30, y: mid.y }, 60);
  out.pinch = { frames: await stopFrames(page), long: await takeLongTasks(page) };
  expect((await arrangementViewState(page)).pxPerBeat, "the pinch zoomed").not.toBe(panned.pxPerBeat);

  for (const [kind, r] of Object.entries(out)) {
    const line =
      `${label} ${kind}: frames median ${median(r.frames).toFixed(1)} ms, p95 ${p95(r.frames).toFixed(1)} ms (${r.frames.length}); ` +
      `long tasks ${r.long.length}, longest ${Math.max(0, ...r.long).toFixed(0)} ms`;
    console.log(line);
    test.info().annotations.push({ type: "perf", description: line });
  }
  return out;
}

test.describe("on a Pixel 7", () => {
  test.use(PIXEL);

  test("pan and pinch frame times, a mid-range phone (CPU 4× slower) against the same gestures unthrottled @frames", async ({ page }) => {
    test.setTimeout(300_000);
    await launchMobile(page);
    await buildPerfProject(page, 16);
    await page.waitForTimeout(1000);
    const base = await gestures(page, "pixel 7 unthrottled");
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
    const phone = await gestures(page, "pixel 7 cpu 4x");
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });

    const host = hostHasHeadroom();
    console.log(`mobile perf: ${host.why}: timing ${host.ok ? "asserted" : "not asserted (host oversubscribed)"}`);
    if (!host.ok) return;
    for (const kind of ["pan", "pinch"] as const) {
      // Not a frame in five slower than the unthrottled p95 plus a 4× CPU's share of it.
      expect(p95(phone[kind].frames), `${kind} p95`).toBeLessThan(Math.max(4 * p95(base[kind].frames), 50));
      // No single main-thread stall of a quarter second (a React render of the whole arrangement).
      expect(Math.max(0, ...phone[kind].long), `${kind} longest task`).toBeLessThan(250);
    }
  });
});

memoryTest.describe("on a Pixel 7, memory", () => {
  memoryTest.use(PIXEL);

  memoryTest(`memory: the tab stays under ${BUDGET_MB} MB with a 16-track project`, async ({ freshPage: page }) => {
    test.setTimeout(180_000);
    await launchMobile(page);
    expect(await page.evaluate(() => crossOriginIsolated && typeof SharedArrayBuffer === "function")).toBe(true);
    await buildPerfProject(page, 16);
    await gestures(page, "pixel 7 (memory run)");
    const uncollected = await tabMemory(page);
    test.skip(uncollected === null, "no /proc here: the renderer's memory is not measured");
    // What the page holds: its garbage collected (the workers' and the worklet's heaps are small).
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("HeapProfiler.enable");
    await cdp.send("HeapProfiler.collectGarbage");
    await cdp.detach();
    // The freed memory goes back to the system a little later (sooner on an idle host): the
    // lowest of a few readings over 3 s.
    let bytes = uncollected!;
    for (let i = 0; i < 6; i++) {
      await page.waitForTimeout(500);
      bytes = Math.min(bytes, (await tabMemory(page))!);
    }
    const mb = (n: number) => (n / 1048576).toFixed(1);
    const line = `pixel 7 memory: ${mb(bytes)} MB after a gc, ${mb(uncollected!)} MB before (the renderer processes: page, workers, worklet, wasm)`;
    console.log(line);
    test.info().annotations.push({ type: "perf", description: line });
    expect(bytes, "after a gc").toBeLessThan(BUDGET_MB * 1048576);
    expect(uncollected!, "before a gc").toBeLessThan(BUDGET_UNCOLLECTED_MB * 1048576);
  });
});

/** A mono 16-bit 44.1 kHz WAV of `seconds`: a slow sweep (waveforms with detail at every zoom). */
function longWav(seconds: number): Buffer {
  const rate = 44_100;
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
  for (let i = 0; i < frames; i++) {
    const t = i / rate;
    const v = 0.6 * Math.sin(2 * Math.PI * (110 + 20 * t) * t) * (0.5 + 0.5 * Math.sin(t));
    b.writeInt16LE(Math.round(32767 * v), 44 + i * 2);
  }
  return b;
}

memoryTest.describe("on a Pixel 7, long audio (#640)", () => {
  memoryTest.use(PIXEL);

  memoryTest("memory: the tab stays under budget with four 5-minute audio clips", async ({ freshPage: page }) => {
    test.setTimeout(300_000);
    await launchMobile(page);
    type Doc = { clips: Record<string, { content: { type: string; media?: string } }>; media: Record<string, { name: string }> };
    const doc = () => page.evaluate(() => (window as unknown as { __ether: { state(): { project: Doc } } }).__ether.state().project);
    /** Audio clips of the four files. */
    const longClips = async () => {
      const p = await doc();
      const media = new Set(Object.entries(p.media).flatMap(([id, m]) => (m.name.startsWith("long ") ? [id] : [])));
      return Object.values(p.clips).filter((c) => c.content.type === "Audio" && media.has(c.content.media!)).length;
    };
    // On disk (in the test's output folder): Playwright passes at most 50 MB of buffers.
    const audio = longWav(300);
    const files = [1, 2, 3, 4].map((n) => test.info().outputPath(`long ${n}.wav`));
    for (const f of files) writeFileSync(f, audio);
    const chooser = page.waitForEvent("filechooser");
    await page.keyboard.press("ControlOrMeta+i");
    await (await chooser).setFiles(files);
    await expect.poll(longClips, { timeout: 120_000 }).toBe(4);
    await page.waitForTimeout(2000);
    await gestures(page, "pixel 7 (long audio)");
    const bytes = await tabMemory(page);
    test.skip(bytes === null, "no /proc here: the renderer's memory is not measured");
    const line = `pixel 7 memory, 4 × 5 min audio (20 min, ${((files.length * audio.length) / 1048576).toFixed(0)} MB of WAV): ${(bytes! / 1048576).toFixed(1)} MB`;
    console.log(line);
    test.info().annotations.push({ type: "perf", description: line });
    expect(bytes!).toBeLessThan(BUDGET_LONG_AUDIO_MB * 1048576);
  });
});

test.describe("on an iPhone 13 (DPR 3)", () => {
  test.use(IPHONE);

  test("the arranger and ruler canvases render at DPR 2", async ({ page }) => {
    await launchMobile(page);
    expect(await page.evaluate(() => window.devicePixelRatio)).toBe(3);
    const ratio = (sel: string) =>
      page
        .locator(sel)
        .first()
        .evaluate((c: HTMLCanvasElement) => {
          const css = c.getBoundingClientRect();
          return c.width / css.width;
        });
    await expect.poll(() => ratio(".eth-arr-gl__canvas")).toBeCloseTo(2, 1);
    await expect.poll(() => ratio(".eth-arr canvas:not(.eth-arr-gl__canvas)")).toBeCloseTo(2, 1);
  });
});
