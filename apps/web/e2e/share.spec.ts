// Sharing end to end on the web build (docs/SHARING.md §12, node `share-integration`): two
// browser contexts, each with its own in-browser engine, through the Node signaling adapter
// (`services/signal/test/server.ts`, the same `RoomCore` as the Cloudflare Durable Object).
// Nothing is mocked: the host's hub runs in its controller Worker, the data channel is a real
// `RTCPeerConnection` pair on loopback, and the joiner's copy is a real offline copy in OPFS.
//
// 1. Diego shares, copies the edit link; Ada opens it in her browser → landing → Continue in
//    browser → "Diego invites you to …" → Join. Ada gets Diego's project.
// 2. Edits both ways, then chat both ways.
// 3. Diego closes his tab: Ada sees "Diego is offline", keeps editing her offline copy.
//    Diego opens Ethereal again and reopens the project: sharing resumes, Ada reconnects
//    by herself ("Diego is back") and her offline edit reaches Diego.
// 4. Diego stops sharing: Ada is told, her copy stays.
//
// No sleeps: every step waits on UI or engine state.
import { spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Browser, type BrowserContext, type Page } from "@playwright/test";
import type { Project, ProjectSummary } from "@/generated";
import { createTrack, playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

// Real host candidates (no mDNS `.local` names: the devbox may not resolve them).
test.use({
  viewport: { width: 1440, height: 900 },
  launchOptions: {
    args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--disable-features=WebRtcHideLocalIpsWithMdns"],
  },
});

interface Handle {
  state(): { project: Project | null; projects: ProjectSummary[] };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const trackNames = async (page: Page): Promise<string[]> => {
  const p = await project(page);
  return p ? Object.values(p.tracks).map((t) => `${t.name}:${t.id}`).sort() : [];
};

/** `ShareState.type` as the top bar's share slot shows it. */
const shareState = (page: Page): Promise<string | null> => page.locator('[data-feature="share"]').getAttribute("data-state");

// --- the signaling service ------------------------------------------------------------------

let signal: ChildProcess | null = null;
let signalUrl = "";

/**
 * `node services/signal/test/server.ts --port 0` → `signal: http://127.0.0.1:<port>`. The
 * adapter uses TypeScript parameter properties, which Node's default type stripping refuses:
 * `--experimental-transform-types`.
 */
async function startSignal(): Promise<{ url: string; child: ChildProcess }> {
  const child = spawn(process.execPath, ["--experimental-transform-types", "--no-warnings", "services/signal/test/server.ts", "--port", "0"], {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "inherit"],
  });
  const url = await new Promise<string>((ok, fail) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /signal: (http:\/\/\S+)/.exec(buf);
      if (m) ok(m[1]!);
    });
    child.on("exit", (code) => fail(new Error(`the signal adapter exited (${code})`)));
  });
  return { url, child };
}

test.beforeAll(async () => {
  ({ url: signalUrl, child: signal } = await startSignal());
});

test.afterAll(() => {
  signal?.kill();
});

// --- people ---------------------------------------------------------------------------------

/**
 * A browser profile with a name and Settings > Advanced > Signaling server set.
 * `realLaunch`: behave like a user's browser, not a WebDriver one (the app then shows the
 * project screen at launch, which joining from a link must not leave over the join).
 */
async function person(browser: Browser, name: string, { realLaunch = false } = {}): Promise<BrowserContext> {
  const ctx = await browser.newContext({ permissions: ["clipboard-read", "clipboard-write"] });
  if (realLaunch) await ctx.addInitScript(() => Object.defineProperty(Navigator.prototype, "webdriver", { get: () => false }));
  await ctx.addInitScript(
    ([n, s]) => {
      localStorage.setItem("eth.share.identity", JSON.stringify({ name: n, color: null }));
      localStorage.setItem("eth-share-settings", JSON.stringify({ signalUrl: s }));
    },
    [name, signalUrl] as const,
  );
  return ctx;
}

async function boot(page: Page): Promise<void> {
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

/** The popover's link, copied with the Copy button (the clipboard as a user would). */
async function copyEditLink(page: Page): Promise<string> {
  const pop = page.getByTestId("share-popover");
  await expect(pop.getByLabel("Edit link")).toHaveValue(/\/join\//, { timeout: 20_000 });
  await pop.getByTestId("share-copy").click();
  await expect(pop.getByTestId("share-copy")).toHaveText("Copied");
  return page.evaluate(() => navigator.clipboard.readText());
}

/** The link as this test's web server serves it (the app makes links for its public origin). */
function local(link: string, page: Page): string {
  const u = new URL(link);
  const base = new URL(page.url());
  return `${base.origin}${u.pathname}${u.search}${u.hash}`;
}

async function closeLeftPane(page: Page): Promise<void> {
  const pane = page.locator('section[data-pane="left"]');
  if (await pane.isVisible()) await page.getByRole("button", { name: "Chat", exact: true }).click();
  await expect(pane).toBeHidden();
}

async function sendChat(page: Page, text: string): Promise<void> {
  await page.keyboard.press("ControlOrMeta+Shift+M");
  const input = page.getByTestId("chat-input");
  await expect(input).toBeFocused();
  await input.fill(text);
  await input.press("Enter");
}

test("share → link → join → edits and chat both ways → host away and back → stop", async ({ browser }) => {
  test.setTimeout(300_000);
  const errors: string[] = [];
  const watch = (p: Page) => p.on("pageerror", (e) => errors.push(`${e.message}`));
  const diegoCtx = await person(browser, "Diego");
  const adaCtx = await person(browser, "Ada", { realLaunch: true });
  let diego = await diegoCtx.newPage();
  const ada = await adaCtx.newPage();
  watch(diego);
  watch(ada);
  if (process.env.SHARE_E2E_DEBUG) {
    diego.on("console", (m) => console.log(`[diego] ${m.text()}`));
    ada.on("console", (m) => console.log(`[ada] ${m.text()}`));
    for (const [who, p] of [["diego", diego], ["ada", ada]] as const) {
      p.on("websocket", (ws) => {
        if (!ws.url().includes("/v1/")) return;
        console.log(`[${who}] ws ${ws.url()}`);
        ws.on("framesent", (f) => console.log(`[${who}] → ${String(f.payload).slice(0, 200)}`));
        ws.on("framereceived", (f) => console.log(`[${who}] ← ${String(f.payload).slice(0, 200)}`));
        ws.on("close", () => console.log(`[${who}] ws closed`));
        ws.on("socketerror", (e) => console.log(`[${who}] ws error ${e}`));
      });
    }
  }
  await boot(diego);

  // --- 1. Diego shares a project with one track; Ada joins with the edit link.
  await createTrack(diego, "Midi");
  const songName = (await project(diego))!.settings.name;
  await diego.getByTestId("share-button").click();
  await expect(diego.getByTestId("share-popover")).toBeVisible();
  const link = await copyEditLink(diego);
  expect(link).toMatch(/\/join\/[\w-]{22}\?s=[^#]+#1[\w-]{22}$/);
  expect(decodeURIComponent(new URL(link).searchParams.get("s")!)).toBe(signalUrl);
  await expect(diego.getByTestId("session-pill")).toHaveAccessibleName(/^Session: Live/);
  await diego.keyboard.press("Escape");

  await ada.goto(local(link, diego));
  expect(await ada.evaluate(() => navigator.webdriver), "Ada's browser takes the real launch path").toBe(false);
  await ada.getByTestId("join-landing").getByRole("button", { name: "Continue in browser" }).click();
  const ready = ada.getByTestId("join-ready");
  await expect(ready).toContainText(`Diego invites you to ${songName}`, { timeout: 30_000 });
  await expect(ada.getByRole("dialog").getByText("You can edit")).toBeVisible();
  await expect.poll(() => ada.evaluate(() => location.href)).not.toContain("#");
  await ada.getByTestId("join-accept").click();
  await expect(ada.getByText(`You're in ${songName} with Diego`)).toBeVisible({ timeout: 30_000 });
  // Joining from a link at launch: the project screen never covers the joined project.
  await expect(ada.getByRole("dialog", { name: "Projects" })).toHaveCount(0);
  await expect.poll(() => shareState(ada)).toBe("Joined");
  await expect.poll(() => project(ada).then((p) => p?.settings.name)).toBe(songName);
  const start = await trackNames(diego);
  await expect.poll(() => trackNames(ada)).toEqual(start);
  await expect(diego.locator(".eth-toast").filter({ hasText: "Ada joined" })).toBeVisible({ timeout: 10_000 });

  // --- 2. Edits both ways, chat both ways.
  await createTrack(ada, "Audio");
  await expect.poll(() => trackNames(diego), { timeout: 10_000 }).toHaveLength(start.length + 1);
  await createTrack(diego, "Midi");
  await expect.poll(() => trackNames(ada), { timeout: 10_000 }).toHaveLength(start.length + 2);
  expect(await trackNames(ada)).toEqual(await trackNames(diego));

  await sendChat(diego, "Hi Ada, the drums are on track 1");
  await expect(ada.locator(".eth-toast").filter({ hasText: "the drums are on track 1" })).toBeVisible({ timeout: 10_000 });
  await sendChat(ada, "Got it, adding bass");
  await expect(diego.getByTestId("chat-message")).toHaveCount(2, { timeout: 10_000 });
  await expect(diego.getByTestId("chat-message").last()).toContainText("Ada");
  // The chat pane floats over the track headers: close it.
  await closeLeftPane(diego);
  await closeLeftPane(ada);

  // --- 3. Diego closes his tab: Ada keeps an offline copy and edits it.
  // (He saves first: the hub's copy is the master, and a tab closed with unsaved edits
  // would start the next session from the last save. The app warns before leaving.)
  const pid = (await project(diego))!.id;
  await diego.keyboard.press("ControlOrMeta+s");
  await expect.poll(() => diego.evaluate(() => (window as unknown as { __ether: { state(): { dirty: boolean } } }).__ether.state().dirty)).toBe(false);
  await diego.close();
  await expect(ada.getByTestId("share-offline-banner")).toContainText("Diego is offline", { timeout: 30_000 });
  await expect(ada.getByTestId("session-pill")).toHaveAccessibleName(/Diego offline/);
  await createTrack(ada, "Midi");
  const adaOffline = await trackNames(ada);
  expect(adaOffline).toHaveLength(start.length + 3);

  // Diego opens Ethereal again and reopens the shared project: sharing resumes by itself.
  diego = await diegoCtx.newPage();
  watch(diego);
  await boot(diego);
  if ((await project(diego))!.id !== pid) {
    await diego.getByRole("button", { name: "Projects" }).click();
    const screen = diego.getByRole("dialog", { name: "Projects" });
    await screen.getByRole("button", { name: songName, exact: false }).first().dblclick();
  }
  await expect.poll(() => project(diego).then((p) => p?.id), { timeout: 30_000 }).toBe(pid);
  await expect(diego.getByTestId("session-pill")).toHaveAccessibleName(/^Session: Live/, { timeout: 30_000 });

  // Ada reconnects without doing anything; her offline track reaches Diego.
  await expect(ada.getByTestId("share-offline-banner")).toHaveCount(0, { timeout: 60_000 });
  await expect(ada.locator(".eth-toast").filter({ hasText: "Diego is back" })).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => trackNames(diego), { timeout: 30_000 }).toEqual(adaOffline);
  await createTrack(diego, "Audio");
  await expect.poll(() => trackNames(ada), { timeout: 10_000 }).toHaveLength(adaOffline.length + 1);

  // --- 4. Diego stops sharing: Ada keeps her copy.
  await diego.getByTestId("session-pill").click();
  await diego.getByTestId("share-popover").getByRole("button", { name: "Stop sharing" }).click();
  await diego.getByRole("dialog", { name: /^Stop sharing/ }).getByRole("button", { name: "Stop sharing" }).click();
  await expect(diego.getByTestId("share-button")).toBeVisible({ timeout: 10_000 });
  await expect(ada.locator(".eth-toast").filter({ hasText: "stopped sharing" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => shareState(ada), { timeout: 10_000 }).toBe("Off");
  expect(await trackNames(ada)).toHaveLength(adaOffline.length + 1);

  expect(errors).toEqual([]);
});
