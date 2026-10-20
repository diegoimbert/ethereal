// The mixer on touch screens: the transport's view switch shows channel strips (the
// master pinned right); a drag on a fader changes the volume relative to where it was, slower
// with the finger off to the side (fine mode); mute / solo by tap; back to the tracks.
import { expect, test, type Page } from "@playwright/test";
import { IPAD, IPHONE, launchMobile, settled, shot, touchscreen } from "./mobile";
import { createTrack } from "./ui";

const mixer = (page: Page) => page.getByTestId("mobile-mixer");
const volumeOf = (page: Page, id: string) =>
  page.evaluate(
    (t) =>
      (window as unknown as { __ether: { state(): { project: { tracks: Record<string, { mixer: { volume: number; mute: boolean } }> } } } }).__ether.state()
        .project.tracks[t]!.mixer,
    id,
  );

test.describe("phone", () => {
  test.use(IPHONE);

  test("strips with the master pinned; faders drag relative (fine off-axis); mute by tap; back to the tracks", async ({ page }) => {
    await launchMobile(page);
    const a = await createTrack(page, "Audio");
    await createTrack(page, "Midi");
    await page.getByTestId("view-switch").tap();
    await expect(mixer(page)).toBeVisible();
    await expect(mixer(page).locator(".eth-mmixer__strips .eth-mstrip")).toHaveCount(2);
    await expect(mixer(page).locator(".eth-mmixer__master .eth-mstrip")).toHaveCount(1);
    await settled(page);
    await shot(page, "mixer");
    const touch = await touchscreen(page);
    const fader = mixer(page).locator(`.eth-mstrip[data-track="${a.id}"] [role="slider"][aria-orientation="vertical"]`);
    const f = (await fader.boundingBox())!;
    const v0 = (await volumeOf(page, a.id)).volume;
    // Touching the fader low down doesn't jump it: only the drag counts.
    const start = { x: f.x + f.width / 2, y: f.y + f.height * 0.9 };
    await touch.drag(start, { x: start.x, y: start.y - 40 }, { rest: 0 });
    const v1 = (await volumeOf(page, a.id)).volume;
    expect(v1).toBeGreaterThan(v0);
    // The same drag with the finger 200 px off to the side: a tenth of the change.
    const s2 = { x: f.x + f.width / 2, y: f.y + f.height * 0.5 };
    await touch.start([{ ...s2, id: 1 }]);
    await touch.move([{ x: s2.x - 200, y: s2.y, id: 1 }]);
    for (let i = 1; i <= 8; i++) await touch.move([{ x: s2.x - 200, y: s2.y + (40 * i) / 8, id: 1 }]);
    await touch.end();
    const v2 = (await volumeOf(page, a.id)).volume;
    expect(v2).toBeLessThan(v1);
    expect(v1 - v2).toBeLessThan((v1 - v0) * 0.3);
    // Mute by tap.
    await mixer(page)
      .getByRole("button", { name: `mute ${a.name}` })
      .tap();
    await expect.poll(async () => (await volumeOf(page, a.id)).mute).toBe(true);
    // Back to the tracks.
    await page.getByTestId("view-switch").tap();
    await expect(mixer(page)).toHaveCount(0);
    await expect(page.getByTestId("arrangement-content")).toBeVisible();
  });
});

test.describe("tablet", () => {
  test.use(IPAD);

  test("wider strips", async ({ page }) => {
    await launchMobile(page);
    await createTrack(page, "Audio");
    await page.getByTestId("view-switch").tap();
    const strip = mixer(page).locator(".eth-mmixer__strips .eth-mstrip").first();
    expect((await strip.boundingBox())!.width).toBeGreaterThanOrEqual(96);
    await settled(page);
    await shot(page, "mixer-tablet");
  });
});
