// Screenshots of the join flow for the PR (not part of the regular suite):
// `JOIN_SHOTS=<dir> npx playwright test join.shots`.
import { expect, test, type Page } from "@playwright/test";

const dir = process.env.JOIN_SHOTS;
test.skip(!dir, "set JOIN_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1440, height: 900 } });

const ROOM = "AbCdEfGhIjKlMnOpQrStUv";
const KEY = "10123456789_-abcdefghij";

type Share = { command(c: unknown): unknown; failJoin(r: string, m: string): void; state: unknown; host: { emit(e: unknown): void } };

async function open(page: Page, theme: "dark" | "light") {
  await page.addInitScript((t) => {
    (window as unknown as { __etherMockShare: boolean }).__etherMockShare = true;
    localStorage.setItem("eth-theme", t);
  }, theme);
  await page.goto(`/join/${ROOM}#${KEY}`);
  await expect(page.getByTestId("join-landing")).toBeVisible();
}

/** Show a join stage as the engine would (`ShareState::Joining`). */
async function stage(page: Page, s: unknown) {
  await page.evaluate((st) => {
    const share = (window as unknown as { __etherShare: Share }).__etherShare;
    share.state = { type: "Joining", stage: st };
    share.host.emit({ type: "Share", event: { type: "State", state: share.state } });
  }, s);
}

for (const theme of ["dark", "light"] as const) {
  test(`join flow shots (${theme})`, async ({ page }) => {
    await open(page, theme);
    await page.screenshot({ path: `${dir}/${theme}-1-landing.png` });
    await page.getByRole("button", { name: "Open in the app" }).click();
    await expect(page.getByText("Don't have the app?")).toBeVisible();
    await page.screenshot({ path: `${dir}/${theme}-2-landing-download.png` });
    await page.getByRole("button", { name: "Continue in browser" }).click();
    await expect(page.getByTestId("join-ready")).toBeVisible({ timeout: 30_000 });
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${dir}/${theme}-3-ready.png` });
    if (theme === "light") return;
    const invite = {
      host: { name: "Diego", color: 0x5cffe8 },
      project: "01J0000000000000000000000P",
      project_name: "Song",
      role: "Edit",
      online: [
        { name: "Diego", color: 0x5cffe8 },
        { name: "Tom", color: 0xff94a6 },
        { name: "Lea", color: 0xffb454 },
      ],
      local_copy: true,
    };
    await stage(page, { type: "Ready", invite });
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${dir}/${theme}-4-ready-copy.png` });
    await stage(page, { type: "Connecting" });
    await page.screenshot({ path: `${dir}/${theme}-5-connecting.png` });
    await stage(page, { type: "Syncing", received_bytes: 6_400_000, total_bytes: 10_000_000 });
    await page.screenshot({ path: `${dir}/${theme}-6-syncing.png` });
    await stage(page, { type: "HostOffline", local_copy: "01J0000000000000000000000P" });
    await page.screenshot({ path: `${dir}/${theme}-7-host-offline.png` });
    await stage(page, { type: "Failed", reason: "InvalidInvite", message: "" });
    await page.screenshot({ path: `${dir}/${theme}-8-failed.png` });
  });
}

test("damaged link landing (dark)", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("eth-theme", "dark"));
  await page.goto(`/join/${ROOM}#1short`);
  await expect(page.getByRole("alert")).toBeVisible();
  await page.screenshot({ path: `${dir}/dark-9-landing-damaged.png` });
});
