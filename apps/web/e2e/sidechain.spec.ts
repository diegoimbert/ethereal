// sidechain: set a compressor's sidechain source from its device header on the web build
// (real wasm engine + controller). Two audio tracks; a compressor on the second one; the
// header selector lists the first track (not master, not its own track), sets it, undo
// restores "No sidechain" in one step, and clearing works. Devices without a sidechain input
// (Delay) show no selector.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, launch, newProject, openDeviceTab, pickOption, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(
    () => (window as unknown as { __ether: Handle }).__ether.state().project,
  );

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const count = (o: object) => Object.keys(o).length;

test("set a compressor sidechain from the device header", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await launch(page);
  await expect(playButton(page)).toBeVisible({
    timeout: 30_000,
  });
  await expect
    .poll(() => project(page).then((p) => p !== null), { timeout: 30_000 })
    .toBe(true);

  const name = `Sidechain ${Date.now()}`;
  await newProject(page, name);
  await expect(page.getByTestId("project-name")).toHaveText(name);
  const baseTracks = count((await doc(page)).tracks);

  // Two audio tracks: the first is the source, the second gets the compressor.
  for (let i = 1; i <= 2; i++) {
    await createTrack(page, "Audio");
    await expect
      .poll(async () => count((await doc(page)).tracks))
      .toBe(baseTracks + i);
  }
  const audio = Object.values((await doc(page)).tracks)
    .filter((t) => t.kind === "Audio")
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  const [source, target] = audio as [(typeof audio)[0], (typeof audio)[0]];

  await openDeviceTab(page, target.name);
  const chainOf = async () =>
    Object.values((await doc(page)).devices).filter(
      (d) => d.track === target.id,
    );
  for (const [i, type] of (["Compressor", "Delay"] as const).entries()) {
    await addDevice(page, type);
    await expect.poll(async () => (await chainOf()).length).toBe(i + 1);
  }
  const chain = await chainOf();
  const comp = chain.find(
    (d) => d.kind.type === "Builtin" && d.kind.device.type === "Compressor",
  )!;
  const delay = chain.find((d) => d.id !== comp.id)!;
  const panel = (id: string) => page.locator(`[data-device="${id}"]`);

  const selector = panel(comp.id).getByRole("combobox", {
    name: `Sidechain source for ${comp.name}`,
  });
  await expect(selector).toBeVisible();
  await expect(selector).toHaveText("No sidechain");
  await expect(
    panel(delay.id).getByRole("combobox", { name: /Sidechain source/ }),
  ).toHaveCount(0);

  // Only other non-master tracks are offered.
  await selector.click();
  const labels = await page
    .locator(`[id="${await selector.getAttribute("aria-controls")}"]`)
    .getByRole("option")
    .allTextContents();
  await page.keyboard.press("Escape");
  await expect(selector).toHaveAttribute("aria-expanded", "false");
  expect(labels).toContain(source.name);
  expect(labels).not.toContain(target.name);
  expect(labels).not.toContain("Master");

  const sidechainOf = async () => (await doc(page)).devices[comp.id]?.sidechain;
  await pickOption(page, selector, source.name);
  await expect.poll(sidechainOf).toBe(source.id);
  await expect(selector).toHaveText(source.name);

  // One undo step.
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(sidechainOf).toBeNull();
  await expect(selector).toHaveText("No sidechain");

  await pickOption(page, selector, source.name);
  await expect.poll(sidechainOf).toBe(source.id);
  await pickOption(page, selector, "No sidechain");
  await expect.poll(sidechainOf).toBeNull();

  expect(errors).toEqual([]);
});
