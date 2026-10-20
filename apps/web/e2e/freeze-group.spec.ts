// Freeze Group on the web build (real wasm controller in the Worker, engine in the worklet):
// group two MIDI tracks (Cmd+G), freeze the group from its header menu (the group plays one
// render; its children are locked: lock badge, disabled mute/solo, edits rejected), unfreeze,
// freeze again and flatten the group into one audio track, then undo the flatten.
//
// `FREEZE_GROUP_SHOTS=<dir>` also saves PR screenshots (1440x900, dark; one light shot).
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { clipMenu } from "./clips";
import { createTrack, launch, newProject, playButton, selectTrack } from "./ui";

const shots = process.env.FREEZE_GROUP_SHOTS;
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

const header = (page: Page, name: string) => page.getByRole("group", { name: `${name} track` });

async function trackMenu(page: Page, name: string, item: string): Promise<void> {
  await header(page, name).click({ button: "right", position: { x: 6, y: 10 } });
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}

async function emptyClip(page: Page, t: Track): Promise<void> {
  const before = Object.keys((await doc(page)).clips).length;
  await page.locator(`[data-lane="${t.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(before + 1);
}

test("freeze a group, its children lock, unfreeze, flatten and undo", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project !== null), {
      timeout: 30_000,
    })
    .toBe(true);
  await newProject(page, `Freeze Group ${Date.now()}`);
  const keys = await createTrack(page, "Midi");
  const bass = await createTrack(page, "Midi");
  await emptyClip(page, keys);
  await emptyClip(page, bass);

  // Group them (select both, Cmd+G).
  await selectTrack(page, keys.name);
  const bassHeader = header(page, bass.name);
  const box = (await bassHeader.boundingBox())!;
  await bassHeader.click({ position: { x: 6, y: box.height / 2 }, modifiers: ["Shift"] });
  await page.keyboard.press("ControlOrMeta+g");
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Group")).toBe(true);
  const group = Object.values((await doc(page)).tracks).find((t) => t.kind === "Group")!;
  await shot(page, "01-group");

  // --- Freeze Group from the group header menu -------------------------------------------
  await trackMenu(page, group.name, "freeze group");
  await expect.poll(async () => (await doc(page)).tracks[group.id]!.freeze !== undefined, { timeout: 60_000 }).toBe(true);
  await expect(page.getByRole("progressbar")).toHaveCount(0);
  const frozen = await doc(page);
  expect(frozen.tracks[keys.id]!.freeze).toBeUndefined();
  expect(frozen.media[frozen.tracks[group.id]!.freeze!.media]!.file).toMatch(/^media\//);
  // The group shows the pressed snowflake; its children are locked.
  await expect(page.getByRole("button", { name: `unfreeze ${group.name}` })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(`.eth-arr-row--frozen[data-track="${group.id}"]`)).toHaveCount(1);
  for (const child of [keys, bass]) {
    await expect(page.locator(`.eth-arr-row--inert[data-track="${child.id}"]`)).toHaveCount(1);
    await expect(page.getByRole("img", { name: `frozen with group ${group.name}` }).first()).toBeVisible();
    await expect(header(page, child.name).getByRole("button", { name: `mute ${child.name}` })).toBeDisabled();
    await expect(header(page, child.name).getByRole("button", { name: `solo ${child.name}` })).toBeDisabled();
  }
  await expect(header(page, group.name).getByRole("button", { name: `mute ${group.name}` })).toBeEnabled();
  await shot(page, "02-frozen-group");

  // A child's menu says why it is locked; its clip cannot be deleted.
  await header(page, keys.name).click({ button: "right", position: { x: 6, y: 10 } });
  await expect(page.getByRole("menuitem", { name: `frozen with group “${group.name}”` })).toBeDisabled();
  await shot(page, "03-child-menu");
  await page.keyboard.press("Escape");
  const clipId = Object.values(frozen.clips).find((c) => c.track === keys.id)!.id;
  await clipMenu(page, clipId);
  await page.getByRole("menuitem", { name: /^delete(?! time)/ }).click();
  await expect.poll(async () => (await doc(page)).clips[clipId]).toBeDefined();

  // The group's menu offers Unfreeze Group / Flatten Group.
  await header(page, group.name).click({ button: "right", position: { x: 6, y: 10 } });
  await expect(page.getByRole("menuitem", { name: "flatten group" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "unfreeze group" })).toBeVisible();
  await shot(page, "04-group-menu");
  await page.getByRole("menuitem", { name: "unfreeze group" }).click();
  await expect.poll(async () => (await doc(page)).tracks[group.id]!.freeze).toBeUndefined();
  await expect(page.locator(".eth-arr-row--inert")).toHaveCount(0);
  await expect(header(page, keys.name).getByRole("button", { name: `solo ${keys.name}` })).toBeEnabled();

  // --- Freeze again, then Flatten Group: one audio track replaces the group ---------------
  await trackMenu(page, group.name, "freeze group");
  await expect.poll(async () => (await doc(page)).tracks[group.id]!.freeze !== undefined, { timeout: 60_000 }).toBe(true);
  await trackMenu(page, group.name, "flatten group");
  // Flatten asks first.
  await page.getByRole("dialog").getByRole("button", { name: "flatten", exact: true }).click();
  await expect.poll(async () => (await doc(page)).tracks[group.id]).toBeUndefined();
  const flat = await doc(page);
  expect(flat.tracks[keys.id]).toBeUndefined();
  expect(flat.tracks[bass.id]).toBeUndefined();
  const audio = Object.values(flat.tracks).find((t) => t.name === group.name && t.kind === "Audio")!;
  expect(audio).toBeDefined();
  const clips = Object.values(flat.clips);
  expect(clips).toHaveLength(1);
  expect(clips[0]!.track).toBe(audio.id);
  expect(clips[0]!.content.type).toBe("Audio");
  await shot(page, "05-flattened");

  // One undo brings the frozen group and its children back.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await doc(page)).tracks[group.id]?.freeze !== undefined).toBe(true);
  expect((await doc(page)).tracks[keys.id]).toBeDefined();

  if (shots) {
    await page.emulateMedia({ colorScheme: "light" });
    await shot(page, "06-frozen-group-light");
  }
  expect(errors).toEqual([]);
});
