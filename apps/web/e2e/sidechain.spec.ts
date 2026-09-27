// sidechain: set a compressor's sidechain source from its device header on the web build
// (real wasm engine + controller). Two audio tracks; a compressor on the second one; the
// header selector lists the first track (not master, not its own track), sets it, undo
// restores "No sidechain" in one step, and clearing works. Devices without a sidechain input
// (Delay) show no selector.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";

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

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({
    timeout: 30_000,
  });
  await expect
    .poll(() => project(page).then((p) => p !== null), { timeout: 30_000 })
    .toBe(true);

  await page.getByRole("button", { name: "Projects" }).click();
  const name = `Sidechain ${Date.now()}`;
  await page.getByLabel("New project name").fill(name);
  await page
    .getByRole("dialog", { name: "Projects" })
    .getByRole("button", { name: "New" })
    .click();
  await expect(page.getByTestId("project-name")).toHaveText(name);
  const baseTracks = count((await doc(page)).tracks);

  // Two audio tracks: the first is the source, the second gets the compressor.
  for (let i = 1; i <= 2; i++) {
    await page.getByRole("button", { name: "+ Audio track" }).click();
    await expect
      .poll(async () => count((await doc(page)).tracks))
      .toBe(baseTracks + i);
  }
  const audio = Object.values((await doc(page)).tracks)
    .filter((t) => t.kind === "Audio")
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  const [source, target] = audio as [(typeof audio)[0], (typeof audio)[0]];

  await page.getByRole("group", { name: `${target.name} track` }).click();
  await page.getByRole("tab", { name: "Devices" }).click();
  const chainOf = async () =>
    Object.values((await doc(page)).devices).filter(
      (d) => d.track === target.id,
    );
  for (const [i, type] of (["Compressor", "Delay"] as const).entries()) {
    await page.getByLabel("Add device").selectOption(type);
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
  await expect(selector).toHaveValue("");
  await expect(
    panel(delay.id).getByRole("combobox", { name: /Sidechain source/ }),
  ).toHaveCount(0);

  // Only other non-master tracks are offered.
  const labels = await selector.locator("option").allTextContents();
  expect(labels).toContain(source.name);
  expect(labels).not.toContain(target.name);
  expect(labels).not.toContain("Master");

  const sidechainOf = async () => (await doc(page)).devices[comp.id]?.sidechain;
  await selector.selectOption({ label: source.name });
  await expect.poll(sidechainOf).toBe(source.id);
  await expect(selector).toHaveValue(source.id);

  // One undo step.
  await page.getByRole("button", { name: "Undo" }).click();
  await expect.poll(sidechainOf).toBeNull();
  await expect(selector).toHaveValue("");

  await selector.selectOption({ label: source.name });
  await expect.poll(sidechainOf).toBe(source.id);
  await selector.selectOption({ label: "No sidechain" });
  await expect.poll(sidechainOf).toBeNull();

  expect(errors).toEqual([]);
});
