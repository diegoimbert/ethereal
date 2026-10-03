// groups-buses on the web build (real wasm engine + controller): Cmd+G groups the selected
// tracks into a folding group track (a bus), Ungroup puts them back, a VCA is created from
// the track menu and shows what it controls, and the inspector routes one track's output
// into another's input with a tap point. The engine takes every graph (VCAs and taps
// travel in the binary codec v3) without an error.
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { createTrack, launch, newProject, pickOption, playButton, selectTrack } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const trackNamed = async (page: Page, name: string): Promise<Track | undefined> =>
  Object.values((await doc(page)).tracks).find((t) => t.name === name);

test("group, ungroup, VCA and track-to-track input", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Groups ${Date.now()}`);

  const a = await createTrack(page, "Audio");
  const b = await createTrack(page, "Audio");
  const c = await createTrack(page, "Midi");

  // Select a and b (shift-click), then Cmd/Ctrl+G.
  await selectTrack(page, a.name);
  const bHeader = page.getByRole("group", { name: `${b.name} track` });
  const box = (await bHeader.boundingBox())!;
  await bHeader.click({ position: { x: 6, y: box.height / 2 }, modifiers: ["Shift"] });
  await page.keyboard.press("ControlOrMeta+g");
  await expect.poll(async () => (await trackNamed(page, "Group"))?.kind).toBe("Group");
  const group = (await trackNamed(page, "Group"))!;
  await expect.poll(async () => (await doc(page)).tracks[a.id]?.parent).toBe(group.id);
  expect((await doc(page)).tracks[b.id]?.parent).toBe(group.id);
  expect((await doc(page)).tracks[c.id]?.parent).toBeNull();
  // It folds like any group.
  await page.getByRole("button", { name: "Fold Group" }).click();
  await expect(page.getByRole("group", { name: `${a.name} track` })).toHaveCount(0);
  await page.getByRole("button", { name: "Unfold Group" }).click();
  await expect(page.getByRole("group", { name: `${a.name} track` })).toBeVisible();

  // Ungroup from the group's menu: the tracks go back to the top level, in order.
  await page.getByRole("group", { name: "Group track" }).click({ button: "right", position: { x: 6, y: 10 } });
  await page.getByRole("menuitem", { name: /^Ungroup/ }).click();
  await expect.poll(async () => (await doc(page)).tracks[group.id]).toBeUndefined();
  expect((await doc(page)).tracks[a.id]?.parent).toBeNull();

  // A VCA for the MIDI track, from its menu.
  await page.getByRole("group", { name: `${c.name} track` }).click({ button: "right", position: { x: 6, y: 10 } });
  await page.getByRole("menuitem", { name: "New VCA for Track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Vca")).toBe(true);
  const vca = Object.values((await doc(page)).tracks).find((t) => t.kind === "Vca")!;
  await expect.poll(async () => (await doc(page)).tracks[c.id]?.vca).toBe(vca.id);
  await expect(page.getByTestId("vca-lane")).toContainText(c.name);

  // b takes a's signal (inspector "Input"), then switches the tap to Pre FX.
  await selectTrack(page, b.name);
  await pickOption(page, `${b.name} input`, `From ${a.name}`);
  await expect.poll(async () => (await doc(page)).tracks[b.id]?.input).toEqual({ type: "Track", track: a.id, tap: "PostFader" });
  expect((await doc(page)).tracks[b.id]?.monitor).toBe("In");
  await pickOption(page, `${b.name} input tap`, "Pre FX");
  await expect.poll(async () => (await doc(page)).tracks[b.id]?.input).toEqual({ type: "Track", track: a.id, tap: "PreFx" });

  // Undo restores the previous tap (one step per edit).
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await doc(page)).tracks[b.id]?.input).toEqual({ type: "Track", track: a.id, tap: "PostFader" });

  expect(errors.filter((e) => !/favicon/.test(e))).toEqual([]);
});
