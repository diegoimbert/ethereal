// The user docs' screenshots (docs-site/content/<section>/images), taken from the real app so
// they can be regenerated: `node scripts/docs/screenshots.mjs` (it sets DOCS_SHOTS to the
// content dir; skipped otherwise). JPEG at 1x keeps each one well under 200 KB.
import { expect, test, type Page } from "@playwright/test";
import { arrangerApi } from "./clips";
import { IPHONE, launchMobile, settled } from "./mobile";
import { installProbe, send, ulid } from "./perfProject";
import { launch } from "./ui";

const dir = process.env.DOCS_SHOTS;

async function shot(page: Page, file: string): Promise<void> {
  await settled(page);
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${dir}/${file}`, type: "jpeg", quality: 72 });
}

/** A small song: four MIDI tracks with a few clips of notes. Resolves with the first clip. */
async function buildSong(page: Page): Promise<string> {
  await installProbe(page);
  const names = ["drums", "bass", "keys", "lead"];
  let n = 0;
  for (const [t, name] of names.entries()) {
    const track = ulid(1, t);
    await send(page, { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name, color: null, parent: null, before: null } });
    for (const [start, length] of [
      [0, 8],
      [8 + (t % 2) * 4, 8 - (t % 2) * 4],
      [16, 8],
    ] as const) {
      const id = ulid(3, n);
      await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track, start, length, name: null } });
      await send(page, {
        domain: "Note",
        command: {
          type: "Add",
          clip: id,
          notes: Array.from({ length: 16 }, (_, k) => ({
            id: ulid(4, n * 100 + k),
            pitch: 48 + t * 5 + ((k * 7) % 12),
            velocity: 0.5 + (k % 4) * 0.12,
            start: (k * length) / 16,
            duration: length / 20,
          })),
        },
      });
      n++;
    }
  }
  return ulid(3, 0);
}

test.describe("docs screenshots", () => {
  test.skip(!dir, "DOCS_SHOTS=<docs-site/content> to capture (scripts/docs/screenshots.mjs)");
  test.setTimeout(120_000);

  test("desktop: the editor and the piano roll", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.addInitScript(() => localStorage.setItem("eth-theme", "dark"));
    await launch(page, "my song");
    const clip = await buildSong(page);
    await expect.poll(() => arrangerApi(page, "clipRect", clip)).not.toBeNull();
    await shot(page, "basics/images/the-screen.jpg");
    // A click on a MIDI clip opens it in the piano roll.
    const r = (await arrangerApi(page, "clipRect", clip))!;
    await page.mouse.click(r.x + r.width / 2, r.y + 6);
    await expect(page.getByTestId("piano-roll")).toBeVisible();
    await shot(page, "making-music/images/piano-roll.jpg");
  });

  test.describe("phone", () => {
    test.use({ ...IPHONE });
    test("the mobile beta", async ({ page }) => {
      await page.addInitScript(() => localStorage.setItem("eth-theme", "dark"));
      await launchMobile(page);
      const clip = await buildSong(page);
      await expect.poll(() => arrangerApi(page, "clipRect", clip)).not.toBeNull();
      await shot(page, "mobile/images/phone.jpg");
    });
  });
});
