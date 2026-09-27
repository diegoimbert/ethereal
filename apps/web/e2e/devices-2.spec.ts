// devices-2: the roadmap v2 effects (EQ, Reverb, Limiter, Utility) on the web build, against
// the real wasm engine. Inserts all four on an audio track playing a demo sample, checks
// their generic param panels (from the engine descriptors), edits a choice, a toggle and
// two knobs, then plays: with the limiter's ceiling at -24 dB and +24 dB input gain, the
// track meter stays at the ceiling.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
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

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** Drag a knob vertically by `dy` pixels (negative = up = increase). */
async function dragKnob(page: Page, knob: Locator, dy: number) {
  await knob.scrollIntoViewIfNeeded();
  const box = (await knob.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(x, y + (dy * i) / 10);
  await page.mouse.up();
}

test("EQ, reverb, limiter and utility on an audio track", async ({ page }) => {
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
  const name = `Devices 2 ${Date.now()}`;
  await page.getByLabel("New project name").fill(name);
  await page
    .getByRole("dialog", { name: "Projects" })
    .getByRole("button", { name: "New" })
    .click();
  await expect(page.getByTestId("project-name")).toHaveText(name);
  const baseTracks = count((await doc(page)).tracks);

  // Audio track with a looping demo sample.
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create audio track" }).click();
  await expect
    .poll(async () => count((await doc(page)).tracks))
    .toBe(baseTracks + 1);
  const audio = Object.values((await doc(page)).tracks).find(
    (t) => t.kind === "Audio",
  )!;
  // The sample browser is a pane opened from the rail; pinned, it doesn't cover the lanes.
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Pin Library" }).click();
  await page
    .getByRole("tablist", { name: "Locations" })
    .getByRole("tab", { name: "Browser library" })
    .click();
  await page
    .getByRole("list", { name: "Files" })
    .getByRole("button", { name: "Demo Samples" })
    .click();
  const sample = page.getByRole("button", {
    name: "Bass Loop 120.wav",
    exact: true,
  });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), {
    targetPosition: { x: 5, y: 20 },
  });
  await expect
    .poll(
      async () =>
        Object.values((await doc(page)).clips).filter(
          (c) => c.track === audio.id,
        ).length,
      { timeout: 20_000 },
    )
    .toBe(1);

  // Insert the four devices.
  // Selecting the track opens the inspector with its devices.
  await page.getByRole("group", { name: `${audio.name} track` }).click();
  const chainOf = async () =>
    Object.values((await doc(page)).devices)
      .filter((d) => d.track === audio.id)
      .sort((a, b) => (a.order < b.order ? -1 : 1));
  for (const [i, type] of (
    ["Eq", "Reverb", "Limiter", "Utility"] as const
  ).entries()) {
    await page.getByRole("combobox", { name: "Add device" }).click();
    await page.locator(`[role="option"][data-value="${type}"]`).click();
    await expect.poll(async () => (await chainOf()).length).toBe(i + 1);
  }
  const chain = await chainOf();
  expect(
    chain.map((d) =>
      d.kind.type === "Builtin" ? d.kind.device.type : d.kind.type,
    ),
  ).toEqual(["Eq", "Reverb", "Limiter", "Utility"]);
  const [eq, reverb, limiter, utility] = chain as [
    (typeof chain)[0],
    (typeof chain)[0],
    (typeof chain)[0],
    (typeof chain)[0],
  ];

  // Param panels come from the engine descriptors.
  const panel = (id: string) => page.locator(`[data-device="${id}"]`);
  await expect(panel(eq.id).getByRole("slider", { name: "Freq" })).toHaveCount(
    8,
  );
  await expect(
    panel(eq.id).getByRole("slider", { name: "Output" }),
  ).toHaveCount(1);
  for (const name of [
    "Pre-Delay",
    "Size",
    "Decay",
    "Damping",
    "Width",
    "Mix",
  ]) {
    await expect(panel(reverb.id).getByRole("slider", { name })).toBeVisible();
  }
  for (const name of ["Gain", "Ceiling", "Release"]) {
    await expect(panel(limiter.id).getByRole("slider", { name })).toBeVisible();
  }
  await expect(
    panel(utility.id).getByRole("button", { name: "Mono" }),
  ).toBeVisible();

  const paramOf = async (device: string, param: number) =>
    (await doc(page)).devices[device]?.params[param];

  // EQ band 1 type (id 1) → Bell; Utility mono (id 5) on.
  await panel(eq.id).getByRole("combobox", { name: "Type" }).first().click();
  await page.getByRole("option", { name: "Bell", exact: true }).click();
  await expect.poll(() => paramOf(eq.id, 1)).toBe(2);
  await panel(utility.id).getByRole("button", { name: "Mono" }).click();
  await expect.poll(() => paramOf(utility.id, 5)).toBe(1);

  // Limiter: ceiling (id 1) to its minimum, input gain (id 0) to its maximum.
  await dragKnob(
    page,
    panel(limiter.id).getByRole("slider", { name: "Ceiling" }),
    300,
  );
  await expect.poll(() => paramOf(limiter.id, 1)).toBe(-24);
  await dragKnob(
    page,
    panel(limiter.id).getByRole("slider", { name: "Gain" }),
    -300,
  );
  await expect.poll(() => paramOf(limiter.id, 0)).toBe(24);

  // Play: the loud (+24 dB) signal is held at the -24 dB ceiling.
  const ceiling = 10 ** (-24 / 20);
  const transportBar = page.getByRole("toolbar", { name: "Transport" });
  await transportBar.getByRole("button", { name: "Play" }).click();
  await expect
    .poll(() => peakOf(page, audio.id), { timeout: 10_000 })
    .toBeGreaterThan(ceiling * 0.5);
  const peaks: number[] = [];
  for (let i = 0; i < 20; i++) {
    peaks.push(await peakOf(page, audio.id));
    await page.waitForTimeout(50);
  }
  await transportBar.getByRole("button", { name: "Stop" }).first().click();
  expect(Math.max(...peaks)).toBeLessThanOrEqual(ceiling * 1.001);

  expect(errors).toEqual([]);
});
