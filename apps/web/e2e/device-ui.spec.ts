// device-ui: the one shared device renderer against the real wasm engine. The Synth's
// declared layout (oscillator, filter curve, envelope) drives the engine's params by knob,
// by curve drag and by envelope handle; every param control is a MIDI-learn and modulation
// target; the param menu opens its automation lane; a device without a layout gets the
// generic one (leading groups, the rest under "More").
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

async function paramOf(page: Page, device: string, param: number): Promise<number | undefined> {
  return (await project(page)).devices[device]?.params[param];
}

async function drag(page: Page, target: Locator, from: { x: number; y: number }, dx: number, dy: number) {
  const box = (await target.boundingBox())!;
  const x = box.x + from.x;
  const y = box.y + from.y;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(x + (dx * i) / 8, y + (dy * i) / 8);
  await page.mouse.up();
}

async function insert(page: Page, trackId: string, type: string): Promise<string> {
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, type);
  let id = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      id = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === trackId) ?? "";
      return id !== "";
    })
    .toBe(true);
  return id;
}

test("the shared renderer draws and drives a declared layout and the generic one", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  const synth = await insert(page, track.id, "Synth");
  const card = page.locator(`section[data-device="${synth}"]`);
  await expect(card.locator('[data-widget="FilterCurve"]')).toBeVisible();
  await expect(card.locator('[data-widget="Envelope"]')).toBeVisible();
  await expect(card.locator('[data-widget="Oscillator"]')).toBeVisible();
  // Everything is on the panel: nothing folded.
  await expect(card.getByRole("button", { name: /More .* controls/ })).toHaveCount(0);

  // Every param control is a MIDI-learn and modulation target.
  const params = card.locator(".eth-param");
  const n = await params.count();
  expect(n).toBeGreaterThanOrEqual(9);
  for (let i = 0; i < n; i++) {
    await expect(params.nth(i)).toHaveAttribute("data-midi-target", /.+/);
    await expect(params.nth(i)).toHaveAttribute("data-mod-target", new RegExp(`^${synth}:\\d+$`));
  }

  // Cutoff knob (param 6): drag up raises it.
  const cutoff = card.getByRole("slider", { name: "Cutoff" });
  await cutoff.scrollIntoViewIfNeeded();
  const c0 = (await paramOf(page, synth, 6)) ?? 8000;
  await drag(page, cutoff, { x: 12, y: 12 }, 0, -30);
  await expect.poll(() => paramOf(page, synth, 6)).toBeGreaterThan(c0);

  // Filter curve: dragging to the left edge and top sets a low cutoff and high resonance.
  const curve = card.getByTestId("widget-filter-curve");
  await curve.scrollIntoViewIfNeeded();
  const cbox = (await curve.boundingBox())!;
  await drag(page, curve, { x: cbox.width / 2, y: cbox.height / 2 }, -cbox.width / 2 + 4, -cbox.height / 2 + 4);
  await expect.poll(async () => (await paramOf(page, synth, 6)) ?? 8000).toBeLessThan(200);
  await expect.poll(async () => (await paramOf(page, synth, 7)) ?? 0).toBeGreaterThan(80);

  // Envelope: drag the attack handle right.
  const env = card.getByTestId("widget-envelope");
  await env.scrollIntoViewIfNeeded();
  const handle = env.locator('[data-handle="Attack"]');
  const hb = (await handle.boundingBox())!;
  const eb = (await env.boundingBox())!;
  await drag(page, env, { x: hb.x - eb.x + hb.width / 2, y: hb.y - eb.y + hb.height / 2 }, 40, 0);
  await expect.poll(async () => (await paramOf(page, synth, 2)) ?? 5).toBeGreaterThan(5);

  // The param menu opens the automation lane in the arrangement.
  await card.locator('[data-param="6"]').first().click({ button: "right" });
  await page.getByRole("menuitem", { name: "Show automation lane" }).click();
  await expect(page.getByText(/Cutoff/).first()).toBeVisible();

  // Any other built-in renders through the same renderer (declared or generic layout).
  const other = await insert(page, track.id, "PolySynth");
  const ocard = page.locator(`section[data-device="${other}"]`);
  await expect(ocard.locator(".eth-layout").first()).toBeVisible();
  await expect(ocard.locator(".eth-param").first()).toHaveAttribute("data-midi-target", /.+/);

  expect(errors).toEqual([]);
});
