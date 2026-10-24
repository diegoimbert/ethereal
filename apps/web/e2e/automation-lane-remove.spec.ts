// Hiding an automation lane of an open track, through the UI, against the real engine: what
// follows moves up by exactly the layout change, measured before and after (FLIP), on every
// frame of the collapse:
//
// - the rows below the track never go past where they end (no overshoot, no snap back);
// - the lanes under the hidden one move with them, the bar ("+ parameter") and the lanes
//   above it stay put (they used to slide up with the slot, then jump back at the end);
// - for the first, a middle, the last and the only lane, at the default size and compact
//   (a narrow header column, lanes at their minimum height).
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { expectRenderer } from "./lanesFixture";
import { createTrack, headerButton, launch, pickOption } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

/** One frame: tops relative to the track's row top (px). */
interface Frame {
  /** The next track's row (it slides by a transform). */
  next: number;
  /** The automation bar of the track. */
  bar: number;
  /** Each lane on screen, by its target key. */
  lanes: Record<string, number>;
  /** On-screen height of the lane being hidden (0 once gone). */
  hidden: number;
}

interface Samples {
  frames: Frame[];
}

/**
 * Frames of `track`'s rows from before `action` until they stop moving: every animation
 * frame and every style change the lane animation makes (it draws each tween frame by
 * styles), so the frames it drew are seen however slow the machine is.
 */
async function sample(page: Page, track: string, hidden: string, action: () => Promise<void>): Promise<Samples> {
  await page.evaluate(
    ({ id, hidden }) => {
      const w = window as unknown as { __samples: Samples; __sampling: boolean };
      const s: Samples = { frames: [] };
      w.__samples = s;
      w.__sampling = true;
      const row = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`)!;
      const take = () => {
        const r = row();
        const top = r.getBoundingClientRect().top;
        const rel = (el: Element | null | undefined) => (el ? el.getBoundingClientRect().top - top : NaN);
        const lanes: Record<string, number> = {};
        let h = 0;
        for (const l of r.querySelectorAll<HTMLElement>(".eth-auto-row[data-target]")) {
          lanes[l.dataset.target!] = rel(l);
          if (l.dataset.target === hidden) h = l.getBoundingClientRect().height;
        }
        s.frames.push({ next: rel(r.nextElementSibling), bar: rel(r.querySelector(".eth-auto-bar")), lanes, hidden: h });
      };
      take();
      const observer = new MutationObserver(take);
      observer.observe(row().parentElement!, { attributes: true, attributeFilter: ["style"], subtree: true });
      let still = 0;
      const tick = () => {
        const last = JSON.stringify(s.frames[s.frames.length - 1]);
        take();
        still = last === JSON.stringify(s.frames[s.frames.length - 1]) ? still + 1 : 0;
        // Stop after ~0.5 s without change once it has moved.
        if (still > 30 && s.frames.some((f) => f.next !== s.frames[0]!.next)) {
          observer.disconnect();
          w.__sampling = false;
        } else requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    },
    { id: track, hidden },
  );
  await action();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __sampling: boolean }).__sampling), { timeout: 10_000 }).toBe(false);
  return page.evaluate(() => (window as unknown as { __samples: Samples }).__samples);
}

/** Wait until `track`'s row and what follows stop moving (two equal samples, frames apart). */
async function settle(page: Page, track: string): Promise<void> {
  let last = "";
  await expect
    .poll(async () => {
      await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
      const now = await page.evaluate((id) => {
        const r = document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`)!;
        return JSON.stringify([
          r.getBoundingClientRect().height,
          r.nextElementSibling?.getBoundingClientRect().top,
          r.querySelectorAll(".eth-auto-row").length,
        ]);
      }, track);
      const still = now === last;
      last = now;
      return still;
    })
    .toBe(true);
}

const lane = (page: Page, track: string, key: string) => page.locator(`.eth-arr-row[data-track="${track}"] .eth-auto-row[data-target="${key}"]`);

/**
 * Hide `key` and check every frame against the layout before and after (see the file doc).
 * `above`/`below`: the lanes that stay, over and under it.
 */
async function hideAndCheck(page: Page, track: string, key: string, above: string[], below: string[]): Promise<void> {
  await settle(page, track);
  const removed = await lane(page, track, key).evaluate((el) => el.getBoundingClientRect().height);
  const { frames } = await sample(page, track, key, () => lane(page, track, key).getByRole("button", { name: "hide lane" }).click());
  await expect(lane(page, track, key)).toHaveCount(0);
  await settle(page, track);
  const first = frames[0]!;
  const last = frames[frames.length - 1]!;
  // FLIP: the layout change measured before and after is the hidden lane's height.
  const delta = last.next - first.next;
  expect(delta).toBeCloseTo(-removed, 0);
  for (const f of frames) {
    const moved = f.next - first.next;
    // Between where it was and where it ends, every frame: no overshoot.
    expect(moved).toBeLessThanOrEqual(0.5);
    expect(moved).toBeGreaterThanOrEqual(delta - 0.5);
    // The bar and the lanes above stay put; the lanes under it move with the rows below.
    expect(Math.abs(f.bar - first.bar)).toBeLessThanOrEqual(0.5);
    for (const k of above) expect(Math.abs(f.lanes[k]! - first.lanes[k]!)).toBeLessThanOrEqual(0.5);
    for (const k of below) expect(Math.abs(f.lanes[k]! - first.lanes[k]! - moved)).toBeLessThanOrEqual(1);
  }
  // Monotonic: the rows below only ever move up.
  for (let i = 1; i < frames.length; i++) expect(frames[i]!.next).toBeLessThanOrEqual(frames[i - 1]!.next + 0.5);
  for (const k of below) expect(last.lanes[k]! - first.lanes[k]!).toBeCloseTo(delta, 0);
  // Drawn by the tween, not a jump: a frame with the lane partly collapsed (how many
  // in-between frames there are depends on the machine).
  expect(frames.some((f) => f.hidden > 0.5 && f.hidden < removed - 0.5)).toBe(true);
}

const MODES = [
  { name: "default", header: null, short: false },
  { name: "compact: narrow header", header: 180, short: false },
  { name: "compact: short lanes", header: null, short: true },
] as const;

for (const mode of MODES) {
  test(`hiding a lane moves what follows by exactly the layout change (${mode.name}) @frames`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    if (mode.header !== null) await page.addInitScript((w) => localStorage.setItem("eth.arr.headerWidth", String(w)), mode.header);
    await launch(page, `Lane remove ${Date.now()}`);
    const midi = await createTrack(page, "Midi");
    await createTrack(page, "Audio");
    await expectRenderer(page);
    const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id)!;
    const volume = `volume:${midi.id}`;
    const transpose = `param:${synth.id}:1`;
    const pan = `pan:${midi.id}`;

    await (await headerButton(page, midi.name, `Show automation of ${midi.name}`)).click();
    await expect(lane(page, midi.id, volume)).toBeVisible();
    await settle(page, midi.id);
    await pickOption(page, "show parameter", { value: transpose });
    await settle(page, midi.id);
    await pickOption(page, "show parameter", { value: pan });
    await settle(page, midi.id);
    if (mode.short) {
      // Every lane at its minimum height (dragged up past it).
      for (const key of [volume, transpose, pan]) {
        const handle = lane(page, midi.id, key).getByRole("separator");
        const box = (await handle.boundingBox())!;
        await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
        await page.mouse.down();
        await page.mouse.move(box.x + box.width / 2, box.y - 300, { steps: 4 });
        await page.mouse.up();
        await settle(page, midi.id);
        expect(await lane(page, midi.id, key).evaluate((el) => el.getBoundingClientRect().height)).toBeCloseTo(32, 0);
      }
    }

    // A middle lane, then (shown again, last) the last one, the first one and the only one.
    await hideAndCheck(page, midi.id, transpose, [volume], [pan]);
    await pickOption(page, "show parameter", { value: transpose });
    await expect(lane(page, midi.id, transpose)).toBeVisible();
    await hideAndCheck(page, midi.id, transpose, [volume, pan], []);
    await hideAndCheck(page, midi.id, volume, [], [pan]);
    await hideAndCheck(page, midi.id, pan, [], []);
    expect(errors).toEqual([]);
  });
}
