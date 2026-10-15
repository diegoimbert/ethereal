// The track header's level meter (a thin bar on the edge between the header column and the
// lanes, outside the header card since 3e474601) is visible while a track plays: not covered
// by the arranger canvas or anything else, and its fill follows the engine's level. Both
// arranger renderers.
import { expect, test, type Page } from "@playwright/test";
import { addDevice, launch, openDeviceTab, playButton } from "./ui";
import { installProbe, send, ulid } from "./perfProject";

test.use({ viewport: { width: 1440, height: 900 } });

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: { meter(t: string): { peak: [number, number] } | null } }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

test("the header meter shows the track's level and nothing covers it", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, `Header meter ${Date.now()}`);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await installProbe(page);
  const track = ulid(1, 7);
  await send(page, { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name: "Meter", color: null, parent: null, before: null } });
  await openDeviceTab(page, "Meter");
  await addDevice(page, "FmSynth");
  const clip = ulid(7, 1);
  await send(page, { domain: "Clip", command: { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: null } });
  await send(page, {
    domain: "Note",
    command: { type: "Add", clip, notes: [48, 55, 60, 64].map((pitch, k) => ({ id: ulid(8, k), pitch, velocity: 0.9, start: 0, duration: 4 })) },
  });
  await send(page, { domain: "Transport", command: { type: "SetLoopRegion", region: { start: 0, end: 4 } } });
  await send(page, { domain: "Transport", command: { type: "SetLoopEnabled", enabled: true } });
  await playButton(page).click();
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeGreaterThan(0.02);

  const row = page.locator(`.eth-arr-row[data-track="${track}"] > .eth-arr-row__main`);
  const meter = row.locator(":scope > .eth-arr-header__meter");
  const fill = meter.locator(".eth-arr-header__meter-fill");
  if (process.env.METER_SHOTS) {
    await page.screenshot({ path: `${process.env.METER_SHOTS}-${test.info().project.name}.png`, clip: { x: 0, y: 40, width: 420, height: 260 } });
  }
  const box = await meter.boundingBox();
  expect(box && box.width > 0 && box.height > 0).toBe(true);
  // Not covered: the meter is pointer-events: none (hit testing skips it), so it is made
  // hittable for the check; then the topmost element at its center is the meter itself, not a
  // canvas or anything else on top.
  await expect
    .poll(
      () =>
        meter.evaluate((m: HTMLElement) => {
          const r = m.getBoundingClientRect();
          m.style.pointerEvents = "auto";
          const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
          m.style.pointerEvents = "";
          return !!top && m.contains(top);
        }),
      { timeout: 10_000 },
    )
    .toBe(true);
  // The fill rises with the level (scaleY > 0) while playing.
  await expect
    .poll(async () => Number(/scaleY\(([\d.e-]+)\)/.exec((await fill.getAttribute("style")) ?? "")?.[1] ?? 0), { timeout: 10_000 })
    .toBeGreaterThan(0);
});
