// Track header buttons keep fixed slots (#239), both arranger renderers: a group (no record arm)
// and the master (no solo, no arm) leave those buttons' slots empty, so mute and solo line up
// with the MIDI and audio tracks' at the default width, in a narrow column and on short lanes.
//
// No sleeps. `E2E_SHOTS=<dir>` saves PR screenshots.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}
const doc = async (page: Page): Promise<Project> => (await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project))!;

/** Each track header's kind and its mute / solo buttons' left edges (null: no such button). */
const columns = (page: Page) =>
  page.locator(".eth-arr-header[data-track]").evaluateAll((headers) =>
    headers.map((h) => {
      const x = (sel: string) => {
        const el = h.querySelector(sel);
        return el ? Math.round(el.getBoundingClientRect().left) : null;
      };
      const kind = [...h.classList].find((c) => /^eth-arr-header--(midi|audio|group|return|master|vca)$/.test(c));
      return { kind: kind?.replace("eth-arr-header--", ""), mute: x(".eth-arr-header__mute"), solo: x(".eth-arr-header__solo") };
    }),
  );

test("track header: mute and solo line up on group, MIDI and audio rows", async ({ page }, info) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await launch(page, `Header slots ${info.project.name} ${Date.now()}`);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });

  const a = await createTrack(page, "Midi");
  await createTrack(page, "Audio");
  await createTrack(page, "Midi");
  await page.getByRole("group", { name: `${a.name} track` }).click({ position: { x: 6, y: 10 } });
  await page.keyboard.press("ControlOrMeta+g");
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Group")).toBe(true);

  const check = async (what: string) => {
    await expect
      .poll(async () => {
        const kinds = new Set((await columns(page)).map((r) => r.kind));
        return ["group", "midi", "audio"].every((k) => kinds.has(k));
      })
      .toBe(true);
    // Every row's mute at one x, and every solo (the master has none: its slot stays empty).
    await expect.poll(async () => new Set((await columns(page)).map((r) => r.mute)).size, `${what}: mute x`).toBe(1);
    const rows = await columns(page);
    expect(new Set(rows.filter((r) => r.solo !== null).map((r) => r.solo)).size, `${what}, solo x: ${JSON.stringify(rows)}`).toBe(1);
  };
  const shot = async (name: string) => {
    if (process.env.E2E_SHOTS) await page.screenshot({ path: `${process.env.E2E_SHOTS}/${info.project.name}-header-slots-${name}.png` });
  };
  await page.mouse.move(0, 0);
  await check("default");
  await shot("default");

  // A narrow header column (smaller buttons): still in line.
  const resize = page.getByRole("separator", { name: "resize track headers" });
  const box = (await resize.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + 40);
  await page.mouse.down();
  await page.mouse.move(box.x - 60, box.y + 40, { steps: 4 });
  await page.mouse.up();
  await expect(page.locator(".eth-arr-header--narrow").first()).toBeVisible();
  await page.mouse.move(0, 0);
  await check("narrow");
  await shot("narrow");

  // Short (compact) lanes: the buttons over the name, still at fixed slots.
  const lane = page.locator(".eth-arr-lane").first();
  const lb = (await lane.boundingBox())!;
  await page.mouse.move(lb.x + lb.width / 2, lb.y + 10);
  await page.keyboard.down("ControlOrMeta");
  await page.keyboard.down("Shift");
  await expect
    .poll(async () => {
      await page.mouse.wheel(0, 400);
      return page.locator(".eth-arr-header--compact").count();
    })
    .toBeGreaterThan(3);
  await page.keyboard.up("Shift");
  await page.keyboard.up("ControlOrMeta");
  await check("compact");
  await page.locator(".eth-arr-header--group").hover();
  await shot("compact");
});
