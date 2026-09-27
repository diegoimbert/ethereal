// freeze-bounce on the web build (real wasm controller in the Worker, engine in the worklet):
// freeze a MIDI track from its header menu (progress in the header, then the frozen
// toggle), its clips are locked, unfreeze with the toggle, freeze again and flatten it into
// an audio track, bounce that clip to a new track, consolidate.
//
// `FREEZE_SHOTS=<dir>` also saves PR screenshots (1440×900, dark; one light shot).
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { openClip } from "./clips";
import { createTrack, newProject, playButton } from "./ui";

const shots = process.env.FREEZE_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const shot = async (page: Page, name: string) => {
  if (shots) await page.screenshot({ path: `${shots}/${name}.png` });
};

const header = (page: Page, t: Track) => page.getByRole("group", { name: `${t.name} track` });

async function trackMenu(page: Page, t: Track, item: string): Promise<void> {
  await header(page, t).click({ button: "right", position: { x: 6, y: 10 } });
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}

/** A MIDI clip on `t` with a few notes drawn in the piano roll. */
async function clipWithNotes(page: Page, t: Track): Promise<string> {
  const before = Object.keys((await doc(page)).clips).length;
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(before + 1);
  const clip = Object.values((await doc(page)).clips).find((c) => c.track === t.id)!;
  await openClip(page, clip.id);
  const grid = page.getByTestId("piano-roll-grid");
  await expect(grid).toBeVisible();
  const box = (await grid.boundingBox())!;
  const end = (await page.getByTestId("piano-roll-clip-end").boundingBox())!;
  const width = end.x - box.x;
  for (const [fx, fy] of [
    [0.03, 0.4],
    [0.14, 0.45],
    [0.25, 0.5],
    [0.4, 0.42],
  ] as const) {
    await grid.dblclick({ position: { x: width * fx + 2, y: box.height * fy } });
  }
  await expect.poll(async () => Object.values((await doc(page)).notes).filter((n) => n.clip === clip.id).length).toBe(4);
  return clip.id;
}

test("freeze, unfreeze, flatten, bounce and consolidate from the arrangement", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project !== null), {
      timeout: 30_000,
    })
    .toBe(true);
  await newProject(page, `Freeze ${Date.now()}`);
  const keys = await createTrack(page, "Midi");
  const bass = await createTrack(page, "Midi");
  await clipWithNotes(page, keys);
  await clipWithNotes(page, bass);
  // Longer content, so the render's progress shows for a moment.
  const bassClip = Object.values((await doc(page)).clips).find((c) => c.track === bass.id)!;
  const handle = page.locator(`[data-clip-id="${bassClip.id}"] [data-handle="resize-end"]`);
  const hb = (await handle.boundingBox())!;
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(hb.x + 600, hb.y + hb.height / 2, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await doc(page)).clips[bassClip.id]!.length).toBeGreaterThan(8);
  await shot(page, "01-before");
  if (shots) {
    // A long render, so the header progress can be captured.
    await page.locator(`[data-clip-id="${bassClip.id}"] .eth-clip__title`).click();
    for (let i = 1; i <= 16; i++) {
      await page.keyboard.press("ControlOrMeta+d");
      await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === bass.id).length).toBe(i + 1);
    }
  }

  // --- Freeze both from the header menus: the second one queues behind the first -------
  await trackMenu(page, bass, "Freeze Track");
  await trackMenu(page, keys, "Freeze Track");
  // The render is fast in release wasm: the progress bars may already be gone.
  if (shots) {
    await page.getByRole("progressbar").first().waitFor({ timeout: 2_000 }).catch(() => {});
    await shot(page, "02-freezing");
  }
  await expect.poll(async () => (await doc(page)).tracks[bass.id]!.freeze !== undefined, { timeout: 60_000 }).toBe(true);
  await expect.poll(async () => (await doc(page)).tracks[keys.id]!.freeze !== undefined, { timeout: 60_000 }).toBe(true);
  await expect(page.getByRole("progressbar")).toHaveCount(0);
  const toggle = page.getByRole("button", { name: `Unfreeze ${bass.name}` });
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(`.eth-arr-row--frozen[data-track="${bass.id}"]`)).toHaveCount(1);
  const frozen = await doc(page);
  const media = frozen.media[frozen.tracks[bass.id]!.freeze!.media]!;
  expect(media.file).toMatch(/^media\//);
  expect(media.frames).toBeGreaterThan(0);
  await shot(page, "03-frozen");

  // Its clips are locked: the engine rejects edits (the lane stays as it was).
  const clipId = Object.values(frozen.clips).find((c) => c.track === bass.id)!.id;
  await page.locator(`[data-clip-id="${clipId}"] .eth-clip__title`).click({ button: "right" });
  await page.getByRole("menuitem", { name: /^Delete/ }).click();
  await expect.poll(async () => (await doc(page)).clips[clipId]).toBeDefined();

  // The header menu now offers Unfreeze / Flatten.
  await header(page, bass).click({ button: "right", position: { x: 6, y: 10 } });
  await expect(page.getByRole("menuitem", { name: "Flatten Track" })).toBeVisible();
  await shot(page, "04-frozen-menu");
  await page.keyboard.press("Escape");

  // --- Unfreeze with the toggle ----------------------------------------------------------
  await toggle.click();
  await expect.poll(async () => (await doc(page)).tracks[bass.id]!.freeze).toBeUndefined();

  // --- Flatten keys (still frozen) into an audio track -----------------------------------
  await trackMenu(page, keys, "Flatten Track");
  await expect.poll(async () => (await doc(page)).tracks[keys.id]).toBeUndefined();
  const flat = Object.values((await doc(page)).tracks).find((t) => t.name === keys.name && t.kind === "Audio")!;
  expect(flat).toBeDefined();
  const flatClip = Object.values((await doc(page)).clips).find((c) => c.track === flat.id)!;
  expect(flatClip.content.type).toBe("Audio");
  await shot(page, "05-flattened");

  // --- Bounce the flattened clip to a new track (source muted) ---------------------------
  await page.locator(`[data-clip-id="${flatClip.id}"] .eth-clip__title`).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Bounce to New Track" }).click();
  await expect
    .poll(async () => Object.values((await doc(page)).tracks).some((t) => t.name === `${flat.name} Bounce`), { timeout: 60_000 })
    .toBe(true);
  expect((await doc(page)).tracks[flat.id]!.mixer.mute).toBe(true);

  // --- Consolidate the bass clip (MIDI: instant) ------------------------------------------
  const bassNow = (await doc(page)).clips[bassClip.id]!;
  await page.locator(`[data-clip-id="${bassClip.id}"] .eth-clip__title`).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Consolidate" }).click();
  await expect.poll(async () => (await doc(page)).clips[bassClip.id]).toBeUndefined();
  const after = await doc(page);
  const consolidated = Object.values(after.clips).find((c) => c.track === bass.id && c.start === bassClip.start)!;
  expect(consolidated.length).toBe(bassNow.length);
  expect(Object.values(after.notes).filter((n) => n.clip === consolidated.id)).toHaveLength(4);
  await shot(page, "06-bounced");

  if (shots) {
    await page.emulateMedia({ colorScheme: "light" });
    await trackMenu(page, bass, "Freeze Track");
    await expect.poll(async () => (await doc(page)).tracks[bass.id]!.freeze !== undefined, { timeout: 60_000 }).toBe(true);
    await shot(page, "07-frozen-light");
  }
  expect(errors).toEqual([]);
});
