// Multi-clip editing (base-143, owner request, Ableton-style) on the web build, through the
// UI, against the real engine (WasmTransport → controller Worker). Two MIDI clips on two
// tracks, a note drawn in each; selecting both clips (click, shift-click) opens them together
// in the piano roll on the song timeline (a band per clip). ⌘A selects the notes of both,
// a drag moves them all (each stays in its own clip) and one ⌘Z undoes the whole drag;
// ↑ transposes both, one ⌘Z again.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx). `E2E_SHOTS=<dir>` saves PR screenshots (dark,
// plus one light).
import { expect, test, type Page } from "@playwright/test";
import type { Note, Project } from "@/generated";
import { openClip, selectClip } from "./clips";
import { createTrack, newProject, pianoRollNote, pianoRollNotes, playButton } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const notesOf = async (page: Page, clip: string): Promise<Note[]> =>
  Object.values((await doc(page)).notes)
    .filter((n) => n.clip === clip)
    .sort((a, b) => a.start - b.start || a.pitch - b.pitch);

const shot = async (page: Page, name: string) => {
  const dir = process.env.E2E_SHOTS;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

test("several selected MIDI clips edit together in the piano roll; one undo per gesture", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Multi-clip ${Date.now()}`);

  // --- Two MIDI clips on two tracks, at different times, a few notes in each -------------------
  const clips: string[] = [];
  for (const at of [5, 260]) {
    const t = await createTrack(page, "Midi");
    await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: at, y: 30 } });
    await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === t.id).length).toBe(1);
    clips.push(Object.values((await doc(page)).clips).find((c) => c.track === t.id)!.id);
  }
  const [a, b] = clips as [string, string];
  const grid = page.getByTestId("piano-roll-grid");
  for (const [clip, rows] of [
    [a, [0, 3]],
    [b, [-2, 2]],
  ] as const) {
    await openClip(page, clip);
    await expect(grid).toBeVisible();
    await expect(page.getByTestId("piano-roll-clip-bands")).toHaveCount(0);
    const gbox = (await grid.boundingBox())!;
    const body = (await page.locator(".eth-pr__body").boundingBox())!;
    for (const [i, row] of rows.entries()) {
      const before = (await notesOf(page, clip)).length;
      await page.mouse.dblclick(gbox.x + 30 + i * 90, body.y + body.height / 2 + row * 12);
      await expect.poll(async () => (await notesOf(page, clip)).length).toBe(before + 1);
    }
  }
  const a0 = await notesOf(page, a);
  const b0 = await notesOf(page, b);

  // B gets its own color (the inspector's color swatch opens the palette): the editor shows
  // each clip's notes in its color.
  await selectClip(page, b);
  await page.getByRole("region", { name: "inspector" }).getByRole("button", { name: "clip color" }).click();
  await page.getByRole("radiogroup", { name: "clip color" }).getByRole("radio").nth(9).click();
  await expect.poll(async () => (await doc(page)).clips[b]!.color).not.toBeNull();

  // --- Select both clips: the piano roll edits them together ---------------------------------
  await selectClip(page, a);
  await selectClip(page, b, ["Shift"]);
  await expect(page.getByTestId("piano-roll-clip-band")).toHaveCount(2);
  await expect(page.getByTestId("piano-roll-clip-count")).toHaveText("2 clips");
  await expect.poll(async () => new Set((await pianoRollNotes(grid)).map((n) => n.id))).toEqual(new Set([...a0, ...b0].map((n) => n.id)));

  // ⌘A selects every note of both clips, like a plain marquee over them: no section.
  await page.getByTestId("piano-roll-clip-band").first().click();
  await page.keyboard.press("ControlOrMeta+a");
  await expect.poll(async () => (await pianoRollNotes(grid)).filter((n) => n.selected).length).toBe(a0.length + b0.length);
  await expect(page.getByTestId("piano-roll-section")).toHaveCount(0);
  await shot(page, "multi-clip-dark");
  if (process.env.E2E_SHOTS) {
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
    await shot(page, "multi-clip-light");
    await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));
  }

  // --- Drag one note: the notes of both clips move by the same amount, in their own clips ------
  const nb = await pianoRollNote(grid, b0[0]!.id);
  const cx = nb.x + Math.min(nb.width / 2, 8);
  const cy = nb.y + nb.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 20, cy - 24, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => (await notesOf(page, b))[0]!.pitch).toBe(b0[0]!.pitch + 2);
  const a1 = await notesOf(page, a);
  expect(a1.map((n) => n.pitch)).toEqual(a0.map((n) => n.pitch + 2));
  expect(a1.every((n) => n.clip === a)).toBe(true);
  const da = a1[0]!.start - a0[0]!.start;
  const db = (await notesOf(page, b))[0]!.start - b0[0]!.start;
  expect(da).toBeCloseTo(db, 6);

  // One ⌘Z restores both clips.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await notesOf(page, a)).map((n) => [n.start, n.pitch])).toEqual(a0.map((n) => [n.start, n.pitch]));
  expect((await notesOf(page, b)).map((n) => [n.start, n.pitch])).toEqual(b0.map((n) => [n.start, n.pitch]));

  // --- ↑ transposes both clips' notes; one ⌘Z again -------------------------------------------
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await notesOf(page, b)).map((n) => n.pitch)).toEqual(b0.map((n) => n.pitch + 1));
  expect((await notesOf(page, a)).map((n) => n.pitch)).toEqual(a0.map((n) => n.pitch + 1));
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await notesOf(page, a)).map((n) => n.pitch)).toEqual(a0.map((n) => n.pitch));
  expect((await notesOf(page, b)).map((n) => n.pitch)).toEqual(b0.map((n) => n.pitch));

  expect(errors).toEqual([]);
});
