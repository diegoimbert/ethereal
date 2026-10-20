// Click a piano-roll note to hear it (Ableton's MIDI editor preview), on the web
// build against the real wasm engine (WasmTransport → controller Worker → AudioWorklet). A
// MIDI track with the Poly Synth, not armed: holding the mouse on a note sounds on the track
// meter, releasing it goes silent again (no stuck note); with the toolbar's headphones toggle
// off, a press plays nothing.
//
// No sleeps except the "stays silent" check. `E2E_SHOTS=<dir>` saves the PR screenshots.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip } from "./clips";
import { addDevice, createTrack, launch, openDeviceTab, pianoRollNote } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null; armedTracks: string[] };
  meter(track: string): { peak: [number, number] } | null;
}

const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

const armed = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().armedTracks);

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

const notesOf = async (page: Page, clip: string): Promise<Note[]> => Object.values((await doc(page)).notes).filter((n) => n.clip === clip);

const shot = async (target: Locator | Page, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await target.screenshot({ path: `${dir}/${name}.png` });
};

test("clicking a piano-roll note plays it on the track's instrument until release", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Note preview ${Date.now()}`);
  const t = await createTrack(page, "Midi");
  await openDeviceTab(page, t.name);
  await addDevice(page, "PolySynth");
  await expect.poll(async () => Object.values((await doc(page)).devices).some((d) => d.track === t.id)).toBe(true);
  // Not armed, not monitored: only the preview can make it sound.
  expect(await armed(page)).toEqual([]);

  // --- A MIDI clip with one note ----------------------------------------------------------------
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === t.id).length).toBe(1);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!.id;
  await openClip(page, clip);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  const toggle = page.getByTestId("piano-roll-preview");
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  const gbox = (await grid.boundingBox())!;
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  await page.mouse.dblclick(gbox.x + 60, body.y + body.height / 2);
  await expect.poll(async () => (await notesOf(page, clip)).length).toBe(1);
  const note = (await notesOf(page, clip))[0]!;
  // Drawing it auditioned it; the release stopped it.
  await expect.poll(() => peakOf(page, t.id), { timeout: 15_000 }).toBeLessThan(0.001);

  // --- Hold the note: it sounds; release: silence -----------------------------------------------
  const nb = await pianoRollNote(grid, note.id);
  await page.mouse.move(nb.x + nb.width / 2, nb.y + nb.height / 2);
  await page.mouse.down();
  await expect.poll(() => peakOf(page, t.id), { timeout: 15_000 }).toBeGreaterThan(0.01);
  await shot(page, "note-preview-held");
  await page.mouse.up();
  await expect.poll(() => peakOf(page, t.id), { timeout: 15_000 }).toBeLessThan(0.001);
  // A click is not an edit: the note is where it was.
  expect((await notesOf(page, clip))[0]).toMatchObject({ pitch: note.pitch, start: note.start });

  // --- Toggle off: a press plays nothing ----------------------------------------------------------
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await page.mouse.move(nb.x + nb.width / 2, nb.y + nb.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(400);
  expect(await peakOf(page, t.id)).toBeLessThan(0.001);
  await page.mouse.up();
  // Persisted (per browser); back on for the screenshots.
  expect(await page.evaluate(() => localStorage.getItem("ethereal.pianoRoll.preview"))).toBe("off");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await page.mouse.move(0, 0);
  await shot(page.locator(".eth-pr__toolbar"), "note-preview-toolbar-dark");
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
  await shot(page.locator(".eth-pr__toolbar"), "note-preview-toolbar-light");
  await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

test("a black key plays across the keyboard's whole width", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, `Black keys ${Date.now()}`);
  const t = await createTrack(page, "Midi");
  await openDeviceTab(page, t.name);
  await addDevice(page, "PolySynth");
  await expect.poll(async () => Object.values((await doc(page)).devices).some((d) => d.track === t.id)).toBe(true);
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 5, y: 30 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === t.id).length).toBe(1);
  await openClip(page, Object.values((await doc(page)).clips).find((c) => c.track === t.id)!.id);
  const keys = page.getByTestId("piano-roll-keys");
  const kb = (await keys.boundingBox())!;
  const body = (await page.locator(".eth-pr__body").boundingBox())!;
  // A black key in view; press its row near the keyboard's right edge (black keys span the
  // full key width since 7e08499b: the press lands on the black key itself).
  const black = keys.locator(".eth-pr-key--black");
  let bb = null;
  for (const k of await black.all()) {
    const b = await k.boundingBox();
    if (b && b.y > body.y + 10 && b.y + b.height < body.y + body.height - 10) {
      bb = b;
      break;
    }
  }
  expect(bb).not.toBeNull();
  const at = { x: kb.x + kb.width * 0.85, y: bb!.y + bb!.height / 2 };
  const hit = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.className, at);
  expect(String(hit)).toContain("eth-pr-key--black");
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await expect.poll(() => peakOf(page, t.id), { timeout: 15_000 }).toBeGreaterThan(0.01);
  await page.mouse.up();
  await expect.poll(() => peakOf(page, t.id), { timeout: 15_000 }).toBeLessThan(0.001);
});
