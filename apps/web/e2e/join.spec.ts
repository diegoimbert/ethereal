// join-flow (docs/SHARING.md §5, §8.3): the web `/join/` route shows the landing before the
// engine boots; "Continue in browser" boots it and opens the join screen; the key is
// stripped from the address bar. `Share::*` is answered by the UI's MockShare (the engine
// side of sharing is another node), enabled with `window.__etherMockShare`.
import { expect, test, type Page } from "@playwright/test";

const ROOM = "AbCdEfGhIjKlMnOpQrStUv";
const KEY = "10123456789_-abcdefghij";

async function mockShare(page: Page, init: Record<string, string> = {}) {
  await page.addInitScript((storage) => {
    (window as unknown as { __etherMockShare: boolean }).__etherMockShare = true;
    for (const [k, v] of Object.entries(storage)) localStorage.setItem(k, v);
  }, init);
}

/** Requests for the engine's wasm (none until the browser is chosen). */
function wasmRequests(page: Page): string[] {
  const urls: string[] = [];
  page.on("request", (r) => {
    if (r.url().endsWith(".wasm")) urls.push(r.url());
  });
  return urls;
}

const engineBooted = (page: Page) => page.evaluate(() => "__etherEngine" in window);

test("/join/… → landing → Continue in browser → Ready → Join (mock)", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await mockShare(page);
  const wasm = wasmRequests(page);

  await page.goto(`/join/${ROOM}#${KEY}`);
  const landing = page.getByTestId("join-landing");
  await expect(landing.getByRole("heading")).toHaveText("You've been invited to an Ethereal project");
  await expect(landing.getByRole("button", { name: "Open in the app" })).toBeVisible();
  // The landing needs no engine.
  expect(await engineBooted(page)).toBe(false);
  expect(wasm).toEqual([]);

  await landing.getByRole("button", { name: "Continue in browser" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByTestId("join-ready")).toContainText("Mock host invites you to Shared song", { timeout: 30_000 });
  await expect(dialog.getByText("You can edit")).toBeVisible();
  expect(await engineBooted(page)).toBe(true);
  // The key never stays in the address bar once the engine has the invite.
  await expect.poll(() => page.evaluate(() => location.href)).toMatch(/\/$/);
  expect(page.url()).not.toContain(KEY);

  // First join: "Join as".
  await dialog.getByLabel("Your name").fill("Ada");
  await dialog.getByRole("button", { name: "Join" }).click();
  await expect(page.getByText("You're in Shared song with Mock host")).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __etherShare: { state: { type: string }; name: string } }).__etherShare.state.type)).toBe(
    "Joined",
  );
  expect(await page.evaluate(() => localStorage.getItem("eth.share.identity"))).toBe(JSON.stringify({ name: "Ada", color: null }));
  expect(errors).toEqual([]);
});

test("a damaged invite says so on the landing; Open Ethereal boots the app without it", async ({ page }) => {
  await mockShare(page);
  await page.goto(`/join/${ROOM}#1short`);
  await expect(page.getByRole("alert")).toHaveText("The invite link is damaged.");
  await expect(page.getByRole("button", { name: "Continue in browser" })).toHaveCount(0);
  await page.getByRole("button", { name: "Open Ethereal" }).click();
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  expect(new URL(page.url()).pathname).toBe("/");
  expect(page.url()).not.toContain("#");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("a reset or revoked invite shows the join screen's message (pasted link)", async ({ page }) => {
  await mockShare(page);
  await page.goto("/");
  // base-131: the app launches on the project screen, with no project; continue without one.
  const projects = page.getByRole("dialog", { name: "Projects" });
  await expect(projects).toBeVisible({ timeout: 30_000 });
  await page.keyboard.press("Escape");
  await expect(projects).toHaveCount(0);
  // The palette's "Join shared project…" → paste.
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByRole("combobox", { name: "Search commands" }).fill("join shared");
  await page.keyboard.press("Enter");
  const paste = page.getByRole("dialog", { name: "Join with a link" });
  await paste.getByLabel("Invite link").fill("https://example.org/song");
  await paste.getByRole("button", { name: "Join" }).click();
  await expect(paste.getByRole("alert")).toHaveText("This is not an Ethereal invite link.");
  await paste.getByLabel("Invite link").fill(`ethereal://join/${ROOM}#2${KEY.slice(1)}`);
  await paste.getByRole("button", { name: "Join" }).click();
  await expect(paste.getByRole("alert")).toHaveText("This invite needs a newer version of Ethereal.");
  await paste.getByLabel("Invite link").fill(`ethereal://join/${ROOM}#${KEY}`);
  await paste.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("join-ready")).toContainText("Mock host invites you to Shared song");
  // An invite the host reset: the engine's failure is shown with the §8.3 text.
  await page.evaluate(() => {
    const share = (window as unknown as { __etherShare: { state: unknown; command(c: unknown): unknown } }).__etherShare;
    share.command({ type: "Leave" });
    (share as unknown as { failJoin(r: string, m: string): void }).failJoin("InvalidInvite", "unknown room");
  });
  await expect(page.getByRole("alert")).toHaveText("This invite link was reset or is no longer valid. Ask the host for a new one.");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("'Always continue in browser' skips the landing", async ({ page }) => {
  await mockShare(page, { "eth.join.preferBrowser": "1", "eth.share.identity": JSON.stringify({ name: "Ada", color: null }) });
  await page.goto(`/join/${ROOM}#${KEY}`);
  await expect(page.getByTestId("join-ready")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("join-landing")).toHaveCount(0);
  await expect(page.getByLabel("Your name")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
  await page.getByRole("button", { name: "Not now" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("Open in the app offers the download when no app takes the link", async ({ page }) => {
  await mockShare(page);
  await page.goto(`/join/${ROOM}?s=https%3A%2F%2Fsig.example#${KEY}`);
  // No app is registered for ethereal:// here, so the page stays (the deep link itself is
  // unit-tested: ui/src/features/share/join/links.test.ts).
  await page.getByRole("button", { name: "Open in the app" }).click();
  await expect(page.getByText("Don't have the app?")).toBeVisible({ timeout: 5_000 });
  await expect(page.getByRole("link", { name: "Download it" })).toHaveAttribute("href", /releases\/latest/);
  // Still on the landing, key untouched (the app may yet open it).
  await expect(page.getByTestId("join-landing")).toBeVisible();
});
