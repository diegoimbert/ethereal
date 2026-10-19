// Clip envelopes (automation owned by a clip) on the web build, through the UI, against the
// real engine (WasmTransport → controller Worker → AudioWorklet).
//
// 1. MIDI: piano roll → lane picker "Envelope · Mixer / Volume" → double-click draws the
//    first point, creating a clip-owned lane in content time → duplicating the clip copies
//    the envelope with it.
// 2. Audio: warp editor → its envelope strip (opens on Clip Gain) → a point at silence →
//    play with the loop on: the clip is silent while it plays → turning the envelope off
//    brings the sound back.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { AutomationLane, AutomationPoint, Project } from "@/generated";
import { openClip, selectClip } from "./clips";
import { createTrack, expandFolder, launch, newProject, openLibrary, pickOption, playButton, playingButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const clipLanes = async (page: Page, clip: string): Promise<AutomationLane[]> =>
  Object.values((await doc(page)).automation_lanes).filter((l) => l.owner.type === "Clip" && l.owner.clip === clip);

const pointsOf = async (page: Page, lane: string): Promise<AutomationPoint[]> =>
  Object.values((await doc(page)).automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

const positionSeconds = (position: Locator) =>
  position.evaluate((el) => {
    const m = /^(\d+):(\d+)\.(\d+)$/.exec((el.textContent ?? "").trim());
    return m ? Number(m[1]) * 60 + Number(m[2]) + Number(m[3]) / 1000 : NaN;
  });

async function start(page: Page, name: string) {
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `${name} ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(0);
}

test("clip envelopes: draw one in the piano roll; it moves with the clip", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await start(page, "Clip envelope MIDI");

  const midi = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${midi.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;
  await openClip(page, clip.id);
  const roll = page.getByTestId("piano-roll");
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();

  await pickOption(roll, "lane", { value: `env:volume:${midi.id}` });
  const env = roll.getByTestId("clip-envelope");
  await expect(env).toHaveAttribute("data-target", `volume:${midi.id}`);
  const svg = env.getByTestId("automation-lane-svg");
  const box = (await svg.boundingBox())!;
  await svg.dblclick({ position: { x: 4, y: box.height * 0.25 } });
  await expect.poll(async () => (await clipLanes(page, clip.id)).length).toBe(1);
  // The second point takes the envelope's value there; held, it's dragged down.
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.5);
  await page.mouse.down({ clickCount: 2 });
  const [lane] = await clipLanes(page, clip.id);
  expect(lane!.target).toEqual({ type: "TrackVolume", track: midi.id });
  await expect.poll(async () => (await pointsOf(page, lane!.id)).length).toBe(2);
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.75, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await pointsOf(page, lane!.id))[1]!.value).toBeLessThan((await pointsOf(page, lane!.id))[0]!.value);
  const pts = await pointsOf(page, lane!.id);
  expect(pts[0]!.value).toBeGreaterThan(pts[1]!.value);
  await expect(roll.getByTestId("clip-envelope-enabled")).toHaveAttribute("aria-pressed", "true");

  // Duplicate the clip: the copy carries its own envelope, same points.
  await selectClip(page, clip.id);
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(2);
  const copy = Object.values((await doc(page)).clips).find((c) => c.id !== clip.id)!;
  await expect.poll(async () => (await clipLanes(page, copy.id)).length).toBe(1);
  const [copied] = await clipLanes(page, copy.id);
  expect(copied!.target).toEqual(lane!.target);
  expect((await pointsOf(page, copied!.id)).map((p) => [p.time, p.value])).toEqual(pts.map((p) => [p.time, p.value]));

  expect(errors).toEqual([]);
});

test("clip envelopes: an audio clip's gain envelope silences it while it plays", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // The default drawer (320 px) can clip the bottom of the warp editor's envelope strip at
  // this font's line heights: a taller drawer (the shell remembers its size).
  await page.addInitScript(() => localStorage.setItem("eth.shell", JSON.stringify({ bottom: { open: false, pinned: false, size: 420, tab: "piano-roll" } })));
  await start(page, "Clip envelope audio");

  const audio = await createTrack(page, "Audio");
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const clip = Object.values((await doc(page)).clips)[0]!;

  await openClip(page, clip.id);
  const editor = page.getByTestId("warp-editor");
  await expect(editor).toHaveAttribute("data-clip-id", clip.id);
  const strip = editor.getByTestId("clip-envelopes");
  // With no envelope yet the strip opens on the clip's own gain.
  await expect(strip.getByTestId("clip-envelope")).toHaveAttribute("data-target", "clip-gain");
  const svg = strip.getByTestId("automation-lane-svg");
  // The empty envelope's first point takes the current value (no offset: 0 dB); held, it is
  // dragged down past the bottom of the lane: normalized 0, silence.
  // 0 dB on the fader law (-inf..+6 dB): cbrt(10^(-6/20)).
  const zeroDb = Math.cbrt(10 ** (-6 / 20));
  await expect.poll(async () => Number(await svg.getAttribute("data-baseline"))).toBeCloseTo(zeroDb, 9);
  // A click where the editor has settled (the locator waits), then the second press, held.
  await svg.click({ position: { x: 4, y: (await svg.boundingBox())!.height - 1 } });
  await page.mouse.down();
  await expect.poll(async () => (await clipLanes(page, clip.id)).length).toBe(1);
  const [lane] = await clipLanes(page, clip.id);
  expect(lane!.target).toEqual({ type: "ClipGain" });
  expect((await pointsOf(page, lane!.id))[0]!.value).toBeCloseTo(zeroDb, 9);
  const box = (await svg.boundingBox())!;
  await page.mouse.move(box.x + 4, box.y + box.height + 200, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await pointsOf(page, lane!.id)).map((p) => p.value)).toEqual([0]);

  // The picker lists the clip's own targets first, then the track's.
  await pickOption(strip, "clip envelope", { value: "env:clip-transpose" });
  await expect(strip.getByTestId("clip-envelope")).toHaveAttribute("data-target", "clip-transpose");
  await pickOption(strip, "clip envelope", { value: "env:clip-gain" });

  // Play with the loop on (0..16 beats, the clip is 8): silent while the clip plays.
  const transportBar = page.getByRole("toolbar", { name: "transport" });
  await transportBar.getByRole("button", { name: "loop" }).click();
  await expect.poll(async () => (await doc(page)).settings.loop_enabled).toBe(true);
  await playButton(page).click();
  await expect(playingButton(page)).toBeVisible();
  const position = page.getByTestId("position-time");
  const often = { timeout: 15_000, intervals: [50] };
  await expect.poll(() => positionSeconds(position), often).toBeGreaterThan(1);
  await expect.poll(() => positionSeconds(position), often).toBeLessThan(3.5);
  expect(await peakOf(page, audio.id)).toBeLessThan(1e-3);

  // Envelope off: the clip sounds again (next pass of the loop at the latest).
  await strip.getByTestId("clip-envelope-enabled").click();
  await expect.poll(async () => (await doc(page)).automation_lanes[lane!.id]?.enabled).toBe(false);
  await expect.poll(() => peakOf(page, audio.id), { timeout: 20_000, intervals: [50] }).toBeGreaterThan(0.01);
  await playingButton(page).click();

  expect(errors).toEqual([]);
});
