// Add track > MIDI track by a plain click opens the instrument picker for the new track (as
// the drag-and-drop path does); picking an instrument replaces the new track's Synth.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { launch, newProject, playButton } from "./ui";

test.use({ viewport: { width: 1600, height: 900 } });

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);

test("clicking Add track > MIDI track opens the instrument picker; a pick lands on the new track", async ({ page }) => {
  test.setTimeout(90_000);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `Add track pick ${Date.now()}`);

  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create MIDI track" }).click();
  const picker = page.getByRole("dialog", { name: /^choose an instrument for / });
  await expect(picker).toBeVisible();
  // It stays open (the click that created the track does not close it).
  await page.waitForTimeout(300);
  await expect(picker).toBeVisible();

  const midi = Object.values((await project(page))!.tracks).find((t) => t.kind === "Midi")!;
  await expect(picker).toHaveAccessibleName(`choose an instrument for ${midi.name}`);
  if (process.env.PICK_SHOT) await page.screenshot({ path: process.env.PICK_SHOT });
  await picker.getByRole("button", { name: "sampler", exact: true }).click();
  await expect(picker).toBeHidden();
  await expect
    .poll(async () =>
      Object.values((await project(page))!.devices)
        .filter((d) => d.track === midi.id)
        .map((d) => (d.kind.type === "Builtin" ? d.kind.device.type : d.kind.type)),
    )
    .toEqual(["Sampler"]);
});
