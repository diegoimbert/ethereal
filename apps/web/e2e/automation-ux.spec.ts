// automation-ux: automation lanes on the web build, through the UI, against the real engine.
//
// - opening a track's automation animates the row height (layout, not a clip): it grows
//   monotonically and ends at the expected height, and a click on the lane then works;
// - a stepped param (the synth's Transpose, semitones) snaps to whole steps;
// - copy/paste of points (keyboard and the lane's "Paste Here" menu);
// - 64 tracks: the open/close animation keeps a steady frame rate.
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { expect, test, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { createTrack, newProject, pickOption } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

test.use({ viewport: { width: 1440, height: 900 } });

/**
 * On-screen heights of a row (from its top to the top of what follows it: rows below slide
 * by a transform) sampled every frame until it stops changing (after `action`).
 */
async function sampleRowHeights(page: Page, track: string, action: () => Promise<void>): Promise<number[]> {
  await page.evaluate((id) => {
    const w = window as unknown as { __rowHeights: number[]; __sampling: boolean };
    w.__rowHeights = [];
    w.__sampling = true;
    const row = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`);
    const next = () => row()?.nextElementSibling as HTMLElement | null;
    let still = 0;
    const tick = () => {
      const h = (next()?.getBoundingClientRect().top ?? 0) - (row()?.getBoundingClientRect().top ?? 0);
      const last = w.__rowHeights[w.__rowHeights.length - 1];
      w.__rowHeights.push(h);
      still = last === h ? still + 1 : 0;
      // Stop after ~0.5 s without change once it has moved.
      if (still > 30 && w.__rowHeights.some((x) => x !== w.__rowHeights[0])) w.__sampling = false;
      else requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, track);
  await action();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __sampling: boolean }).__sampling), { timeout: 10_000 }).toBe(false);
  return page.evaluate(() => (window as unknown as { __rowHeights: number[] }).__rowHeights);
}

test("automation lanes animate open, snap stepped params, and copy/paste points", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await newProject(page, `Automation ${Date.now()}`);
  const midi = await createTrack(page, "Midi");
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id)!;

  // --- Open: the row grows smoothly to lane + bar + one lane -----------------------------
  const laneHeight = await page.locator(`.eth-arr-row[data-track="${midi.id}"] .eth-arr-row__main`).evaluate((el) => el.getBoundingClientRect().height);
  const heights = await sampleRowHeights(page, midi.id, () => page.getByRole("button", { name: `Show automation of ${midi.name}` }).click());
  const expected = laneHeight + 20 + 64;
  expect(heights[heights.length - 1]).toBeCloseTo(expected, 0);
  const moving = heights.filter((h) => h > laneHeight + 0.5 && h < expected - 0.5);
  expect(moving.length).toBeGreaterThanOrEqual(3); // in-between frames: animated, not a jump
  for (let i = 1; i < heights.length; i++) expect(heights[i]!).toBeGreaterThanOrEqual(heights[i - 1]! - 0.01);

  // A click on the lane works once open: double-click adds a point.
  const volume = page.getByRole("group", { name: "Volume automation" });
  const volumeSvg = volume.getByTestId("automation-lane-svg");
  const box = (await volumeSvg.boundingBox())!;
  expect(box.height).toBeCloseTo(64, 0);
  await volumeSvg.dblclick({ position: { x: 40, y: box.height * 0.3 } });
  await volumeSvg.dblclick({ position: { x: 160, y: box.height * 0.7 } });
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(2);

  // --- Copy both points, paste them at a later time (lane menu "Paste Here") ------------
  await volume.focus();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("ControlOrMeta+C");
  await volumeSvg.click({ button: "right", position: { x: 400, y: box.height / 2 } });
  await page.getByRole("menuitem", { name: "Paste Here" }).click();
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(4);
  const vol = Object.values((await doc(page)).automation_lanes).find((l) => l.target.type === "TrackVolume")!;
  const pts = Object.values((await doc(page)).automation_points)
    .filter((p) => p.lane === vol.id)
    .sort((a, b) => a.time - b.time);
  // Same shape, shifted in time.
  expect(pts[2]!.value).toBeCloseTo(pts[0]!.value, 9);
  expect(pts[3]!.value).toBeCloseTo(pts[1]!.value, 9);
  expect(pts[3]!.time - pts[2]!.time).toBeCloseTo(pts[1]!.time - pts[0]!.time, 9);

  // --- Transpose (semitones) snaps to whole steps -----------------------------------------
  await pickOption(page, "Show parameter", { value: `param:${synth.id}:1` });
  const transpose = page.getByRole("group", { name: /Transpose automation/ });
  const tSvg = transpose.getByTestId("automation-lane-svg");
  await expect(tSvg).toBeVisible();
  const tBox = (await tSvg.boundingBox())!;
  await expect(tSvg.locator(".eth-auto-lane__step").first()).toBeAttached();
  await tSvg.dblclick({ position: { x: 80, y: tBox.height * 0.37 } });
  const tLane = async () => Object.values((await doc(page)).automation_lanes).find((l) => l.target.type === "DeviceParam");
  await expect.poll(async () => (await tLane()) !== undefined).toBe(true);
  const semis = async () => {
    const lane = (await tLane())!;
    const p = Object.values((await doc(page)).automation_points).find((x) => x.lane === lane.id)!;
    return p.value * 48 - 24; // Transpose: -24..24 st, linear
  };
  await expect.poll(semis).toBeCloseTo(Math.round(await semis()), 9);
  const before = Math.round(await semis());
  // Drag the point up by ~3 st: lands exactly on a whole semitone.
  const circle = transpose.locator("circle[data-point]").first();
  const c = (await circle.boundingBox())!;
  await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2);
  await page.mouse.down();
  await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2 - 3.4 * ((tBox.height - 10) / 48), { steps: 5 });
  await expect(transpose.getByTestId("automation-drag-tip")).toContainText("st");
  await page.mouse.up();
  await expect.poll(async () => {
    const s = await semis();
    return Math.abs(s - Math.round(s)) < 1e-9 && Math.round(s) !== before;
  }).toBe(true);

  expect(errors).toEqual([]);
});

// ---- 64 tracks: frame rate while lanes open/close ---------------------------------------

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

test("64 tracks: opening/closing automation keeps the frame rate", async ({ page }) => {
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
    return { script: get("ScriptDuration"), layout: get("LayoutDuration") + get("RecalcStyleDuration"), task: get("TaskDuration") };
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
  console.log(
    `automation-ux perf (64 tracks): ${frames.length} frames, median ${med(frames).toFixed(1)} ms (idle ${med(idleFrames).toFixed(1)} ms), ` +
      `p95 ${p95.toFixed(1)} ms; per frame: script ${perFrame("script").toFixed(2)} ms, style+layout ${perFrame("layout").toFixed(2)} ms, ` +
      `main-thread tasks ${perFrame("task").toFixed(2)} ms`,
  );
  // The frame budget at 60 fps is 16.7 ms: the work per animated frame must fit well within
  // it. (The frame interval itself depends on the machine: a headless browser on a shared
  // box may present at 30 Hz even at rest, so it is compared to the idle rate.)
  expect(med(frames)).toBeLessThanOrEqual(med(idleFrames) * 1.25);
  expect(p95).toBeLessThanOrEqual(med(idleFrames) * 2.5);
  expect(perFrame("script")).toBeLessThan(8);
});
