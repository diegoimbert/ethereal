// Extending a clip to the left past its start on the web build, through the UI,
// against the real engine (WasmTransport → controller Worker → AudioWorklet). A MIDI clip
// with notes, moved to bar 3; its left edge dragged 2 bars left:
// - unlooped: the clip starts 2 bars earlier with empty space before the content (offset
//   -8), the notes keep their song time; dragging back right trims the gap first;
// - looped: the offset wraps into the loop (the loop's tail plays first), the loop region
//   doesn't move.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots (dark
// 1440x900, and a light run of the same flow).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import { clipBodyInk, clipMenu, clipPart, openClip } from "./clips";
import { createTrack, launch, pickOption } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const r3 = (x: number) => +x.toFixed(3);
const clipOf = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;
const bounds = async (page: Page, id: string) => {
  const c = await clipOf(page, id);
  return [r3(c.start), r3(c.length), r3(c.offset)];
};

/** Song positions where the clip's notes start (offset, gap and loop applied). */
async function noteTimes(page: Page, id: string): Promise<number[]> {
  const p = await doc(page);
  const c = p.clips[id]!;
  const notes = Object.values(p.notes).filter((n) => n.clip === id);
  const out: number[] = [];
  const { enabled, start: ls, end: le } = c.looping;
  for (const n of notes) {
    if (!enabled) {
      const rel = n.start - c.offset;
      if (rel >= -1e-6 && rel < c.length - 1e-6) out.push(r3(c.start + rel));
      continue;
    }
    // Looped: first pass from the offset to the loop end, then repetitions of the loop.
    const first = le - c.offset;
    if (n.start >= c.offset - 1e-6 && n.start < le) out.push(r3(c.start + n.start - c.offset));
    if (n.start >= ls - 1e-6 && n.start < le) {
      for (let t = first + (n.start - ls); t < c.length - 1e-6; t += le - ls) out.push(r3(c.start + t));
    }
  }
  return out.sort((a, b) => a - b);
}

const shot = async (target: Page | Locator, name: string, theme: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await target.screenshot({ path: `${dir}/${theme}-${name}.png` });
};

/** Piano roll: zoom out so the 4-beat clip fits, then the client x of a content beat. */
async function rollX(page: Page): Promise<(beat: number) => number> {
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  const left = (sel: string) => page.locator(sel).evaluate((el) => parseFloat((el as HTMLElement).style.left));
  const pxPerBeat = async () => ((await left('[data-testid="piano-roll-clip-end"]')) - (await left(".eth-pr__start"))) / 4;
  const room = (body.width - 64) / 2;
  await page.mouse.move(body.x + 70, body.y + body.height / 2);
  for (let i = 0; i < 60 && (await pxPerBeat()) * 4 > room; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, 60);
    await page.keyboard.up("Control");
  }
  let last = NaN;
  await expect
    .poll(async () => {
      const now = await pxPerBeat();
      const settled = now === last;
      last = now;
      return settled;
    })
    .toBe(true);
  const grid = (await page.getByTestId("piano-roll-grid").boundingBox())!;
  const x0 = await left(".eth-pr__start");
  const ppb = await pxPerBeat();
  return (beat) => grid.x + x0 + beat * ppb;
}

/** Drag from (x, y) by dx px with intermediate moves (the arrangement's pointer drag). */
async function dragBy(page: Page, x: number, y: number, dx: number) {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 3 });
  await page.mouse.move(x + dx, y, { steps: 3 });
  await page.mouse.up();
}

/** Drag a clip's left edge (its resize-start strip) by `beats`. */
async function dragStart(page: Page, clip: string, beats: number, px: number) {
  const h = await clipPart(page, clip, "resize-start");
  await dragBy(page, h.x + h.width / 2, h.y + h.height - 4, beats * px);
}

for (const theme of ["dark", "light"] as const) {
  test(`clip extend left: unlooped adds a gap, looped wraps (${theme})`, async ({ page }) => {
    test.skip(theme === "light" && !process.env.E2E_SHOTS, "the light run only captures screenshots");
    test.setTimeout(150_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    // Nothing is open on launch: create the project on the launch screen.
    await launch(page, `Extend left ${theme} ${Date.now()}`);
    await pickOption(page.locator('[data-feature="arrangement"]'), "grid", "1/4");

    // --- A one-bar MIDI clip with notes at content beats 0 and 2 ---------------------------
    const t = await createTrack(page, "Midi");
    const lane = page.locator(`[data-lane="${t.id}"]`);
    await lane.dblclick({ position: { x: 5, y: 30 } });
    await expect.poll(async () => Object.values((await doc(page)).clips).length).toBe(1);
    const id = Object.values((await doc(page)).clips)[0]!.id;
    await openClip(page, id);
    await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
    await pickOption(page.getByTestId("piano-roll"), "grid", "1/4");
    const x = await rollX(page);
    const body = (await page.locator(".eth-pr__body").boundingBox())!;
    for (const [beat, dy] of [
      [0, 0],
      [2, -40],
    ] as const) {
      const before = Object.values((await doc(page)).notes).length;
      await page.mouse.dblclick(x(beat + 0.1), body.y + body.height / 2 + dy);
      await expect.poll(async () => Object.values((await doc(page)).notes).length).toBe(before + 1);
    }
    const arrangement = page.locator('[data-feature="arrangement"]');
    await arrangement.focus();
    await page.keyboard.press("Escape");

    // --- Move it to bar 3 (beat 8) -----------------------------------------------------------
    const clip = id;
    const px = (await clipPart(page, id)).width / 4;
    const title = await clipPart(page, id, "header");
    await dragBy(page, title.x + title.width / 2, title.y + title.height / 2, 8 * px);
    await expect.poll(() => bounds(page, id)).toEqual([8, 4, 0]);
    expect(await noteTimes(page, id)).toEqual([8, 10]);

    // --- Unlooped: left edge 2 bars left → 8 beats of empty space, notes unchanged ----------
    await dragStart(page, clip, -8, px);
    await expect.poll(() => bounds(page, id)).toEqual([0, 12, -8]);
    expect(await noteTimes(page, id)).toEqual([8, 10]);
    expect(Object.values((await doc(page)).notes).filter((n) => n.clip === id)).toHaveLength(2);
    await expect.poll(() => clipBodyInk(page, id)).toBeGreaterThan(0);
    await shot(page, "1-unlooped-leading-gap", theme);
    // Dragging back right trims the gap first.
    await dragStart(page, clip, 4, px);
    await expect.poll(() => bounds(page, id)).toEqual([4, 8, -4]);
    expect(await noteTimes(page, id)).toEqual([8, 10]);
    // One undo step per drag.
    await arrangement.focus();
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(() => bounds(page, id)).toEqual([0, 12, -8]);
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(() => bounds(page, id)).toEqual([8, 4, 0]);

    // --- Looped: enable the loop (the clip's bar), then drag 1.5 bars left ------------------
    await clipMenu(page, id);
    await page.getByRole("menuitem", { name: /enable loop/ }).click();
    await expect.poll(async () => (await clipOf(page, id)).looping).toEqual({ enabled: true, start: 0, end: 4 });
    await dragStart(page, clip, -6, px);
    // Starts at beat 2 with the loop's last 2 beats (offset wrapped to 2); the loop stays.
    await expect.poll(() => bounds(page, id)).toEqual([2, 10, 2]);
    expect((await clipOf(page, id)).looping).toEqual({ enabled: true, start: 0, end: 4 });
    // Wrapped tail (the note at content 2 → beat 2), then the loop; the original notes stay.
    expect(await noteTimes(page, id)).toEqual([2, 4, 6, 8, 10]);
    await shot(page, "2-looped-extended-left", theme);
    // Two bars left plays the loop twice more from its start.
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(() => bounds(page, id)).toEqual([8, 4, 0]);
    await dragStart(page, clip, -8, px);
    await expect.poll(() => bounds(page, id)).toEqual([0, 12, 0]);
    expect(await noteTimes(page, id)).toEqual([0, 2, 4, 6, 8, 10]);

    expect(errors).toEqual([]);
  });
}
