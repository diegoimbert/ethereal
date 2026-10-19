// Screenshots of the mobile UI (#522) for design review and the gate PR: the arrangement with a
// selected clip (its action bar), the track sheet, the mixer and the piano roll, on a phone in
// portrait and landscape and on a tablet, in the dark and light themes.
//
// `MOBILE_SHOTS=<dir>` to capture (skipped otherwise).
import { expect, test, type Page } from "@playwright/test";
import { arrangerApi } from "./clips";
import { installProbe, send, ulid } from "./perfProject";
import { IPAD, IPHONE, launchMobile, settled, touchscreen } from "./mobile";

const dir = process.env.MOBILE_SHOTS;

/** Four MIDI tracks with a few clips of notes each; resolves with a clip of the first track. */
async function buildScene(page: Page): Promise<string> {
  await installProbe(page);
  const names = ["keys", "bass", "lead", "pad"];
  let n = 0;
  for (const [t, name] of names.entries()) {
    const track = ulid(1, t);
    await send(page, { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name, color: null, parent: null, before: null } });
    for (const [start, length] of [
      [t % 2, 4],
      [6 + t, 4],
      [12, 8],
    ] as const) {
      const id = ulid(3, n);
      await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track, start, length, name: null } });
      await send(page, {
        domain: "Note",
        command: {
          type: "Add",
          clip: id,
          notes: Array.from({ length: 12 }, (_, k) => ({
            id: ulid(4, n * 100 + k),
            pitch: 48 + t * 5 + ((k * 7) % 12),
            velocity: 0.5 + (k % 4) * 0.12,
            start: (k * length) / 12,
            duration: length / 16,
          })),
        },
      });
      n++;
    }
  }
  return ulid(3, 0);
}

async function capture(page: Page, theme: "dark" | "light", device: string): Promise<void> {
  const save = async (name: string) => {
    await settled(page);
    await page.screenshot({ path: `${dir}/${device}-${theme}-${name}.png` });
  };
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launchMobile(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const clip = await buildScene(page);
  const touch = await touchscreen(page);
  await expect.poll(() => arrangerApi(page, "clipRect", clip)).not.toBeNull();
  await page.waitForTimeout(500);
  await save("arrangement");

  const r = (await arrangerApi(page, "clipRect", clip))!;
  const on = { x: r.x + r.width / 2, y: r.y + r.height * 0.7 };
  await touch.tap(on);
  await expect(page.getByTestId("clip-actions")).toBeVisible();
  await save("clip-selected");

  // The track sheet: a tap selects the header, a second one opens it.
  const header = page
    .locator(`.eth-arr__scroll .eth-arr-header[data-track="${ulid(1, 0)}"] .eth-arr-header__name, .eth-arr__scroll .eth-arr-header[data-track="${ulid(1, 0)}"]`)
    .first();
  const hb = (await header.boundingBox())!;
  const at = { x: hb.x + Math.min(hb.width / 2, 40), y: hb.y + 12 };
  await touch.tap(at);
  await touch.sleep(400);
  await touch.tap(at);
  await expect(page.getByTestId("track-sheet")).toBeVisible();
  await save("track-sheet");
  await page.getByTestId("track-sheet").getByRole("button", { name: "close", exact: true }).tap();
  await expect(page.getByTestId("track-sheet")).toHaveCount(0);

  await page.getByTestId("view-switch").tap();
  await expect(page.getByTestId("mobile-mixer")).toBeVisible();
  await save("mixer");
  await page.getByTestId("view-switch").tap();
  await expect(page.getByTestId("mobile-mixer")).toBeHidden();

  await touch.sleep(400);
  await touch.doubleTap(on);
  await expect(page.getByTestId("mobile-editor")).toBeVisible();
  await page.waitForTimeout(300);
  await save("piano-roll");
}

const DEVICES = [
  { name: "phone-portrait", use: IPHONE },
  { name: "phone-landscape", use: { ...IPHONE, viewport: { width: 664, height: 390 } } },
  { name: "tablet", use: IPAD },
] as const;

for (const d of DEVICES) {
  test.describe(d.name, () => {
    test.use(d.use);
    for (const theme of ["dark", "light"] as const) {
      test(`${d.name} ${theme}`, async ({ page }) => {
        test.skip(!dir, "MOBILE_SHOTS=<dir> to capture");
        test.setTimeout(120_000);
        await capture(page, theme, d.name);
      });
    }
  });
}
