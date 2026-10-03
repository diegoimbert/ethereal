// automation-ux (perf): 64 tracks on the web build; opening/closing a track's automation
// keeps a steady frame rate (split from automation-ux.spec.ts: tracing is off here).
//
// - Structural, always asserted: a toggle costs a handful of layout passes (React lays
//   the rows out once per tween; frames only move composited transforms), not one per frame.
// - Timing, asserted when the host has CPU headroom (see `hostHasHeadroom`): animated frame
//   intervals against the same page at rest, and main-thread script time per frame.
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { availableParallelism, loadavg } from "node:os";
import { expect, test, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { newProject } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

// Playwright's trace snapshots walk the DOM on the page's main thread around every action,
// inside the measured window (about 800 ms over the six toggles in a CPU profile on the
// devbox): they would be measured as app jank.
test.use({ viewport: { width: 1440, height: 900 }, trace: "off" });

/**
 * Frame timing is only meaningful when the browser gets the CPU it asks for. On a shared box
 * with more runnable threads than cores (1-min load average above the core count), the
 * commit that introduced this test (902ef3b: 16.7 ms median, 2.8 ms script per frame on an
 * idle laptop) measures 17–117 ms medians, 167–367 ms p95 and 23–70 ms of script per frame,
 * worse than today's code on the same host: the timing assertions would measure the host,
 * not the app. There they are logged and annotated instead, and the structural assertions
 * still apply. `ETHER_E2E_STRICT_PERF=1` asserts the timing anyway.
 */
function hostHasHeadroom(): { ok: boolean; why: string } {
  const load = loadavg()[0]!;
  const cores = availableParallelism();
  const why = `1-min load ${load.toFixed(1)} on ${cores} cores`;
  return { ok: process.env.ETHER_E2E_STRICT_PERF === "1" || load <= cores, why };
}

// Layout passes per toggle (CDP `LayoutCount`): 3.5–3.7 measured (devbox, 50–60 frames over
// the six toggles); a per-frame relayout would be one per animated frame (~10–18).
// Style recalcs are logged only: the per-frame transforms are inline styles, so their count
// follows the frame rate.
const LAYOUTS_PER_TOGGLE = 6;

interface Probe {
  replies: Record<number, { status: string }>;
  post(json: string): void;
}
const ulid = (kind: number, n: number) => `01J${kind}${String(n).padStart(22, "0")}`;
let nextId = 2_000_000_000;

async function send(page: Page, command: Command) {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => (window as unknown as { __probe: Probe }).__probe.post(JSON.stringify({ id, gesture: null, command })),
    [id, command] as const,
  );
  await expect
    .poll(() => page.evaluate((id) => (window as unknown as { __probe: Probe }).__probe.replies[id], id), { timeout: 15_000 })
    .toMatchObject({ status: "Ok" });
}

test("64 tracks: opening/closing automation keeps the frame rate @frames", async ({ page }) => {
  test.setTimeout(180_000);
  await page.goto("/");
  await newProject(page, `Automation perf ${Date.now()}`);
  await page.evaluate(() => {
    const ep = (window as unknown as { __etherEngine: { handles(): { controller: Worker } | null; post(json: string): void } }).__etherEngine;
    const probe: Probe = { replies: {}, post: (json) => ep.post(json) };
    ep.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type !== "server" || !e.data.json) return;
      for (const m of JSON.parse(e.data.json) as { kind: string; body: { id: number; result: { status: string } } }[]) {
        if (m.kind === "Reply") probe.replies[m.body.id] = m.body.result;
      }
    });
    (window as unknown as { __probe: Probe }).__probe = probe;
  });
  // One MIDI track with 8 clips of 16 notes, duplicated to 64.
  const first = ulid(1, 0);
  await send(page, { domain: "Track", command: { type: "Create", id: first, kind: "Midi", name: "perf 0", color: null, parent: null, before: null } });
  for (let c = 0; c < 8; c++) {
    const clip = ulid(3, c);
    await send(page, { domain: "Clip", command: { type: "CreateMidi", id: clip, track: first, start: c * 4, length: 4, name: null } });
    await send(page, {
      domain: "Note",
      command: {
        type: "Add",
        clip,
        notes: Array.from({ length: 16 }, (_, k) => ({ id: ulid(4, c * 16 + k), pitch: 60 + (k % 12), velocity: 100, start: k / 4, duration: 0.25 })),
      },
    });
  }
  for (let t = 1; t < 64; t++) await send(page, { domain: "Track", command: { type: "Duplicate", id: first, new_id: ulid(1, t) } });
  await expect.poll(async () => Object.keys((await doc(page)).tracks).length).toBeGreaterThanOrEqual(64);
  await expect(page.locator(".eth-arr-row")).not.toHaveCount(0);

  // Toggle the first track's automation a few times; record frame intervals, and the main
  // thread's script + layout time per frame (CDP), against the same page at rest.
  const toggle = page.getByRole("button", { name: /Show automation of perf 0|Hide automation of perf 0/ }).first();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const metrics = async () => {
    const { metrics: m } = await cdp.send("Performance.getMetrics");
    const get = (n: string) => m.find((x) => x.name === n)?.value ?? 0;
    return {
      script: get("ScriptDuration"),
      layout: get("LayoutDuration") + get("RecalcStyleDuration"),
      task: get("TaskDuration"),
      layouts: get("LayoutCount"),
      styles: get("RecalcStyleCount"),
    };
  };
  const idleFrames = await page.evaluate(
    () =>
      new Promise<number[]>((done) => {
        const out: number[] = [];
        let last = performance.now();
        const tick = (now: number) => {
          out.push(now - last);
          last = now;
          if (out.length < 40) requestAnimationFrame(tick);
          else done(out.slice(1));
        };
        requestAnimationFrame(tick);
      }),
  );
  const m0 = await metrics();
  await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __rec: boolean };
    w.__frames = [];
    w.__rec = true;
    let last = performance.now();
    const tick = (now: number) => {
      w.__frames.push(now - last);
      last = now;
      if (w.__rec) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  for (let i = 0; i < 6; i++) {
    await toggle.click();
    await page.waitForFunction(() => document.querySelector(".eth-auto-track--animating") === null);
  }
  const frames = await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __rec: boolean };
    w.__rec = false;
    return w.__frames.slice(1);
  });
  const m1 = await metrics();
  const med = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!;
  const p95 = [...frames].sort((a, b) => a - b)[Math.floor(frames.length * 0.95)]!;
  const perFrame = (k: "script" | "layout" | "task") => ((m1[k] - m0[k]) * 1000) / frames.length;
  const perToggle = (k: "layouts" | "styles") => (m1[k] - m0[k]) / 6;
  console.log(
    `automation-ux perf (64 tracks): ${frames.length} frames, median ${med(frames).toFixed(1)} ms (idle ${med(idleFrames).toFixed(1)} ms), ` +
      `p95 ${p95.toFixed(1)} ms; per frame: script ${perFrame("script").toFixed(2)} ms, style+layout ${perFrame("layout").toFixed(2)} ms, ` +
      `main-thread tasks ${perFrame("task").toFixed(2)} ms; per toggle: ${perToggle("layouts").toFixed(1)} layouts, ` +
      `${perToggle("styles").toFixed(1)} style recalcs`,
  );
  // Structural (independent of the CPU share): a toggle costs a handful of layout passes,
  // not one per animated frame.
  expect(perToggle("layouts")).toBeLessThanOrEqual(LAYOUTS_PER_TOGGLE);

  // Timing: the frame budget at 60 fps is 16.7 ms; the work per animated frame must fit well
  // within it. (The frame interval itself depends on the machine: a headless browser on a
  // shared box may present at 30 Hz even at rest, so it is compared to the idle rate.)
  const host = hostHasHeadroom();
  console.log(`automation-ux perf: ${host.why}: timing ${host.ok ? "asserted" : "not asserted (host oversubscribed)"}`);
  if (!host.ok) {
    test.info().annotations.push({ type: "perf", description: `timing not asserted: host oversubscribed (${host.why})` });
    return;
  }
  expect(med(frames)).toBeLessThanOrEqual(med(idleFrames) * 1.25);
  expect(p95).toBeLessThanOrEqual(med(idleFrames) * 2.5);
  expect(perFrame("script")).toBeLessThan(8);
});
