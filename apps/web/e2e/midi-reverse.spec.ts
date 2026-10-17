// Reverse a MIDI clip on the web build, through the UI, against the real engine
// (WasmTransport -> controller Worker): the "R" shortcut mirrors the clip's notes in time
// (one undo step), pressing it again restores them, and with notes selected in the piano
// roll only those notes are mirrored inside their span.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip, selectClip } from "./clips";
import { createTrack, launch, pianoRollNotes, pickOption } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

/** `[start, duration]` of the clip's notes, by start. */
async function notes(page: Page, clip: string): Promise<Array<[number, number]>> {
  const p = (await project(page))!;
  return Object.values(p.notes)
    .filter((n: Note) => n.clip === clip)
    .map((n): [number, number] => [+n.start.toFixed(3), +n.duration.toFixed(3)])
    .sort((a, b) => a[0] - b[0]);
}

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

test("reverse a MIDI clip with R (arrangement), and selected notes with R (piano roll)", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, `Reverse MIDI ${Date.now()}`);
  await pickOption(page.locator('[data-feature="arrangement"]'), "grid", "1/4");

  // A one-bar MIDI clip with notes at content beats 0 and 2.
  const t = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await project(page))!.clips).length).toBe(1);
  const id = Object.values((await project(page))!.clips)[0]!.id;
  await openClip(page, id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await pickOption(page.getByTestId("piano-roll"), "grid", "1/4");
  const x = await rollX(page);
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  for (const [beat, dy] of [
    [0, 0],
    [2, -40],
  ] as const) {
    const before = Object.values((await project(page))!.notes).length;
    await page.mouse.dblclick(x(beat + 0.1), body.y + body.height / 2 + dy);
    await expect.poll(async () => Object.values((await project(page))!.notes).length).toBe(before + 1);
  }
  const original = await notes(page, id);
  expect(original).toHaveLength(2);
  const mirrored = original.map(([s, d]): [number, number] => [+(4 - (s + d)).toFixed(3), d]).sort((a, b) => a[0] - b[0]);
  expect(mirrored).not.toEqual(original);

  // --- Arrangement: select the clip, press R ------------------------------------------------
  await page.locator('[data-feature="arrangement"]').focus();
  await page.keyboard.press("Escape");
  await selectClip(page, id);
  await page.keyboard.press("r");
  await expect.poll(() => notes(page, id)).toEqual(mirrored);
  // One undo step.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => notes(page, id)).toEqual(original);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(() => notes(page, id)).toEqual(mirrored);
  // Twice restores the original.
  await page.keyboard.press("r");
  await expect.poll(() => notes(page, id)).toEqual(original);

  // --- Piano roll: select both notes, R mirrors them inside their span ---------------------
  await openClip(page, id);
  await expect(page.getByTestId("piano-roll-grid")).toBeVisible();
  await page.getByTestId("piano-roll").focus();
  await page.keyboard.press("ControlOrMeta+a");
  // ⌘A selects the notes only (a plain marquee's selection), no section over the clip.
  await expect.poll(async () => (await pianoRollNotes(page.getByTestId("piano-roll-grid"))).filter((n) => n.selected).length).toBe(2);
  await expect(page.getByTestId("piano-roll-section")).toHaveCount(0);
  await page.keyboard.press("r");
  const [s0, d0] = original[0]!;
  const [s1, d1] = original[1]!;
  // Span = first start .. last end.
  const lo = s0!;
  const hi = s1! + d1!;
  const inSpan = [
    [+(lo + hi - (s0! + d0!)).toFixed(3), d0!],
    [+(lo + hi - (s1! + d1!)).toFixed(3), d1!],
  ].sort((a, b) => a[0]! - b[0]!);
  await expect.poll(() => notes(page, id)).toEqual(inSpan);
  expect(errors).toEqual([]);
});
