// templates: track and project templates on the web build (wasm engine, templates in the
// OPFS user library). Screenshots for the PR with `TEMPLATES_SHOTS=<dir>`:
//   TEMPLATES_SHOTS=/tmp/shots npx playwright test templates
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { addDevice, createTrack, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

async function start(page: Page, theme?: string) {
  await page.setViewportSize({ width: 1440, height: 900 });
  if (theme) await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
}

const header = (page: Page, name: string) => page.getByRole("group", { name: `${name} track`, exact: true });

/** A new MIDI track with a Poly Synth and a Delay. */
async function chainTrack(page: Page): Promise<Track> {
  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  await addDevice(page, "PolySynth");
  await addDevice(page, "Delay");
  await expect.poll(async () => Object.values((await project(page)).devices).filter((d) => d.track === track.id).length).toBe(2);
  return track;
}

async function trackMenu(page: Page, name: string, item: string) {
  await header(page, name).click({ button: "right" });
  await page.getByRole("menuitem", { name: item }).click();
}

test("save a track as a template and insert it (one undo step), across a reload", async ({ page }) => {
  await start(page);
  const lead = await chainTrack(page);

  await trackMenu(page, lead.name, "Save as Template…");
  const save = page.getByRole("dialog", { name: "Save track as a template" });
  await expect(save.getByRole("textbox", { name: "Template name" })).toHaveValue(lead.name);
  await save.getByRole("textbox", { name: "Template name" }).fill("Lead Chain");
  await save.getByRole("textbox", { name: "Template tags" }).fill("lead");
  await save.getByRole("button", { name: "Save" }).click();
  await expect(save).toBeHidden();

  // Survives a reload (OPFS user library; the project itself is saved first).
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByRole("status", { name: "Unsaved changes" })).toBeHidden();
  await page.reload();
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(async () => (await project(page))?.tracks[lead.id] !== undefined, { timeout: 15_000 }).toBe(true);
  const before = await project(page);
  await trackMenu(page, lead.name, "Insert Track Template…");
  const insert = page.getByRole("dialog", { name: "Insert track template" });
  await insert.getByRole("textbox", { name: "Search templates" }).fill("lead");
  await insert.getByRole("button", { name: /^Lead Chain/ }).click();
  await expect(insert).toBeHidden();
  let added: Track | undefined;
  await expect
    .poll(async () => {
      const p = await project(page);
      added = Object.values(p.tracks).find((t) => !before.tracks[t.id] && t.name === lead.name);
      return added !== undefined;
    })
    .toBe(true);
  const p = await project(page);
  const devices = Object.values(p.devices).filter((d) => d.track === added!.id);
  expect(devices.map((d) => d.kind.type === "Builtin" && d.kind.device.type).sort()).toEqual(["Delay", "PolySynth"]);

  // One undo step.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => Object.keys((await project(page)).tracks).sort()).toEqual(Object.keys(before.tracks).sort());

  // A factory template from the palette.
  await page.getByRole("button", { name: "Command palette" }).click();
  await page.getByRole("combobox", { name: "Search commands" }).fill("Insert track template");
  await page.getByRole("listbox", { name: "Commands" }).getByRole("option", { name: /Insert track template/ }).click();
  await insert.getByRole("button", { name: /^Reverb Return/ }).click();
  await expect.poll(async () => Object.values((await project(page)).tracks).some((t) => t.kind === "Return" && t.name === "Reverb")).toBe(true);
});

test("new project from a project template and the default", async ({ page }) => {
  await start(page);
  await chainTrack(page);
  const source = await project(page);

  await page.getByRole("button", { name: "Projects" }).click();
  const screen = page.getByRole("dialog", { name: "Projects" });
  await screen.getByRole("button", { name: "Save as template…" }).click();
  const save = page.getByRole("dialog", { name: "Save project as a template" });
  await save.getByRole("textbox", { name: "Template name" }).fill("Band");
  await save.getByRole("button", { name: "Save" }).click();
  await expect(save).toBeHidden();

  // New project (the project screen is still open): pick "Band", and make it the default.
  await screen.getByRole("button", { name: "New project" }).click();
  const picker = screen.getByRole("group", { name: "Project templates" });
  await picker.getByRole("button", { name: "More actions for Band" }).click();
  await page.getByRole("menuitem", { name: "Use for New Projects" }).click();
  await expect(picker.getByRole("button", { name: /^Band/ })).toHaveAttribute("aria-current", "true");
  await screen.getByLabel("New project name").fill("Gig");
  await screen.getByRole("button", { name: "Create" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Gig");
  const gig = await project(page);
  expect(gig.id).not.toBe(source.id);
  expect(Object.values(gig.tracks).map((t) => t.name).sort()).toEqual(Object.values(source.tracks).map((t) => t.name).sort());

  // "Empty project" still available.
  await page.getByRole("button", { name: "Projects" }).click();
  await screen.getByRole("button", { name: "New project" }).click();
  await picker.getByRole("button", { name: "Empty project" }).click();
  await screen.getByLabel("New project name").fill("Blank");
  await screen.getByRole("button", { name: "Create" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Blank");
  expect(Object.keys((await project(page)).tracks)).toHaveLength(1);
});

const shots = process.env.TEMPLATES_SHOTS;
for (const theme of ["dark", "light"]) {
  test(`screenshots: template dialogs (${theme})`, async ({ page }) => {
    test.skip(!shots, "set TEMPLATES_SHOTS=<dir> to capture screenshots");
    await start(page, theme);
    const lead = await chainTrack(page);
    await trackMenu(page, lead.name, "Save as Template…");
    const save = page.getByRole("dialog", { name: "Save track as a template" });
    await save.getByRole("textbox", { name: "Template name" }).fill("Night Lead");
    await save.getByRole("textbox", { name: "Template tags" }).fill("lead, synth");
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-save-${theme}.png` });
    await save.getByRole("button", { name: "Save" }).click();
    await expect(save).toBeHidden();

    await trackMenu(page, lead.name, "Insert Track Template…");
    const insert = page.getByRole("dialog", { name: "Insert track template" });
    await expect(insert.getByRole("button", { name: /^Night Lead/ })).toBeVisible();
    await insert.getByRole("button", { name: /^Vocal Chain/ }).hover();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-insert-${theme}.png` });
    await insert.getByRole("button", { name: /^Drum Bus/ }).click();
    await expect(insert).toBeHidden();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-inserted-${theme}.png` });

    await page.getByRole("button", { name: "Projects" }).click();
    const screen = page.getByRole("dialog", { name: "Projects" });
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-project-screen-${theme}.png` });
    await screen.getByRole("button", { name: "Save as template…" }).click();
    const saveProject = page.getByRole("dialog", { name: "Save project as a template" });
    await saveProject.getByRole("textbox", { name: "Template name" }).fill("Live set");
    await saveProject.getByRole("button", { name: "Save" }).click();
    await expect(saveProject).toBeHidden();
    await screen.getByRole("button", { name: "New project" }).click();
    await expect(screen.getByRole("group", { name: "Project templates" })).toBeVisible();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-new-project-${theme}.png` });
  });
}
