// templates: track and project templates on the web build (wasm engine, templates in the
// OPFS user library). Screenshots for the PR with `TEMPLATES_SHOTS=<dir>`:
//   TEMPLATES_SHOTS=/tmp/shots npx playwright test templates
import { expect, test, type Page } from "@playwright/test";
import type { Project, Track } from "@/generated";
import { addDevice, createTrack, openDeviceTab, launch, openOnLaunch, paletteCommand, projectMenu } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

async function start(page: Page, theme?: string) {
  await page.setViewportSize({ width: 1440, height: 900 });
  if (theme) await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
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

/** The palette's "New project from template…": the "New project" form (name + template). */
async function newFromTemplate(page: Page) {
  await paletteCommand(page, "new project from template…");
  const screen = page.getByRole("dialog", { name: "new project" });
  await expect(screen).toBeVisible();
  return screen;
}

async function trackMenu(page: Page, name: string, item: string) {
  await header(page, name).click({ button: "right" });
  await page.getByRole("menuitem", { name: item }).click();
}

test("save a track as a template and insert it (one undo step), across a reload", async ({ page }) => {
  await start(page);
  const lead = await chainTrack(page);

  await trackMenu(page, lead.name, "save as template…");
  const save = page.getByRole("dialog", { name: "save track as a template" });
  await expect(save.getByRole("textbox", { name: "template name" })).toHaveValue(lead.name);
  await save.getByRole("textbox", { name: "template name" }).fill("Lead Chain");
  await save.getByRole("textbox", { name: "template tags" }).fill("lead");
  await save.getByRole("button", { name: "save" }).click();
  await expect(save).toBeHidden();

  // Survives a reload (OPFS user library; the project itself is saved first).
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByRole("status", { name: "unsaved changes" })).toBeHidden();
  await page.reload();
  // Nothing opens on launch; reopen the project from Recents.
  await openOnLaunch(page);
  await expect.poll(async () => (await project(page))?.tracks[lead.id] !== undefined, { timeout: 15_000 }).toBe(true);
  const before = await project(page);
  await trackMenu(page, lead.name, "insert track template…");
  const insert = page.getByRole("dialog", { name: "insert track template" });
  await insert.getByRole("textbox", { name: "search templates" }).fill("lead");
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
  await page.getByRole("button", { name: "command palette" }).click();
  await page.getByRole("combobox", { name: "search commands" }).fill("Insert track template");
  await page
    .getByRole("listbox", { name: "commands" })
    .getByRole("option", { name: /insert track template/ })
    .click();
  await insert.getByRole("button", { name: /^reverb return/ }).click();
  await expect.poll(async () => Object.values((await project(page)).tracks).some((t) => t.kind === "Return" && t.name === "reverb")).toBe(true);
});

test("new project from a project template and the default", async ({ page }) => {
  await start(page);
  await chainTrack(page);
  const source = await project(page);

  await projectMenu(page, "save project as template…");
  const save = page.getByRole("dialog", { name: "save project as a template" });
  await save.getByRole("textbox", { name: "template name" }).fill("Band");
  await save.getByRole("button", { name: "save" }).click();
  await expect(save).toBeHidden();

  // New project from a template (the palette's form): pick "Band", and make it the default.
  const screen = await newFromTemplate(page);
  const picker = screen.getByRole("group", { name: "project templates" });
  await picker.getByRole("button", { name: "more actions for Band" }).click();
  await page.getByRole("menuitem", { name: "use for new projects" }).click();
  await expect(picker.getByRole("button", { name: /^Band/ })).toHaveAttribute("aria-current", "true");
  await screen.getByLabel("new project name").fill("Gig");
  await screen.getByRole("button", { name: "create" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Gig");
  const gig = await project(page);
  expect(gig.id).not.toBe(source.id);
  expect(
    Object.values(gig.tracks)
      .map((t) => t.name)
      .sort(),
  ).toEqual(
    Object.values(source.tracks)
      .map((t) => t.name)
      .sort(),
  );

  // "Empty project" still available.
  await newFromTemplate(page);
  await picker.getByRole("button", { name: "empty project" }).click();
  await screen.getByLabel("new project name").fill("Blank");
  await screen.getByRole("button", { name: "create" }).click();
  await expect(page.getByTestId("project-name")).toHaveText("Blank");
  expect(Object.keys((await project(page)).tracks)).toHaveLength(1);
});

const shots = process.env.TEMPLATES_SHOTS;
for (const theme of ["dark", "light"]) {
  test(`screenshots: template dialogs (${theme})`, async ({ page }) => {
    test.skip(!shots, "set TEMPLATES_SHOTS=<dir> to capture screenshots");
    await start(page, theme);
    const lead = await chainTrack(page);
    await trackMenu(page, lead.name, "save as template…");
    const save = page.getByRole("dialog", { name: "save track as a template" });
    await save.getByRole("textbox", { name: "template name" }).fill("Night Lead");
    await save.getByRole("textbox", { name: "template tags" }).fill("lead, synth");
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-save-${theme}.png` });
    await save.getByRole("button", { name: "save" }).click();
    await expect(save).toBeHidden();

    await trackMenu(page, lead.name, "insert track template…");
    const insert = page.getByRole("dialog", { name: "insert track template" });
    await expect(insert.getByRole("button", { name: /^Night Lead/ })).toBeVisible();
    await insert.getByRole("button", { name: /^vocal chain/ }).hover();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-insert-${theme}.png` });
    await insert.getByRole("button", { name: /^drum bus/ }).click();
    await expect(insert).toBeHidden();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-inserted-${theme}.png` });

    await page.getByRole("button", { name: "projects" }).click();
    await expect(page.getByRole("menu", { name: "project" })).toBeVisible();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-project-screen-${theme}.png` });
    await page.getByRole("menu", { name: "project" }).getByRole("menuitem", { name: "save project as template…" }).click();
    const saveProject = page.getByRole("dialog", { name: "save project as a template" });
    await saveProject.getByRole("textbox", { name: "template name" }).fill("Live set");
    await saveProject.getByRole("button", { name: "save" }).click();
    await expect(saveProject).toBeHidden();
    const screen = await newFromTemplate(page);
    await expect(screen.getByRole("group", { name: "project templates" })).toBeVisible();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${shots}/templates-new-project-${theme}.png` });
  });
}
