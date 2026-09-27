// Listen on a peer, end to end with two real web apps (docs/COLLAB.md §9; nodes
// `stream-host` × `stream-listen`): Ada hosts from her browser (worklet stream tap → web
// sender), Bob listens (receiver, stream clock → playhead) through a local
// `ether-collab-relay` (host/STUN candidates only; TURN is experimental).
//
// both join → Bob sees Ada can host → Bob listens from Ada's chip menu → media flows (Bob's
// peer connection receives packets) → Ada plays: Bob's transport and playhead follow → Bob's
// Stop stops Ada, his second Stop returns her to the start: his playhead follows the stop
// and seek anchors → Ada lists Bob as a listener → Bob stops listening: Ada's list empties.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project, TransportState } from "@/generated";
import { playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-stream-${Math.random().toString(36).slice(2, 8)}`;

// Same-browser WebRTC on loopback: expose host candidates instead of mDNS names.
test.use({
  launchOptions: {
    args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--disable-features=WebRtcHideLocalIpsWithMdns"],
  },
});

interface Handle {
  state(): { project: Project | null; transport: TransportState | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
const transportState = (page: Page): Promise<TransportState | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().transport);
const bars = async (page: Page): Promise<number> => Number((await page.getByTestId("position-bars").textContent())?.split(".")[0] ?? 0);

/** Build `ether-collab-relay` and return the binary path (cargo tells where it is). */
function buildRelay(): string {
  const out = execFileSync("cargo", ["build", "-p", "ether-collab", "--bin", "ether-collab-relay", "--message-format=json"], {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-collab-relay" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the ether-collab-relay binary");
}

let relay: ChildProcess | null = null;
let relayUrl = "";

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const bin = buildRelay();
  const child = spawn(bin, ["--port", "0", "--token", TOKEN], {
    env: { ...process.env, RUST_LOG: "warn" },
    stdio: ["ignore", "pipe", "inherit"],
  });
  relay = child;
  relayUrl = await new Promise<string>((resolveUrl, reject) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) resolveUrl(m[1]!);
    });
    child.on("exit", (code) => reject(new Error(`ether-collab-relay exited (${code})`)));
  });
});

test.afterAll(() => {
  relay?.kill();
});

async function open(page: Page): Promise<void> {
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function join(page: Page, name: string) {
  await page.getByTestId("collab-button").click();
  await page.getByLabel("Relay address").fill(relayUrl);
  await page.getByLabel("Session").fill(SESSION);
  await page.getByLabel("Your name").fill(name);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });
}

/** Audio packets Bob's peer connections received (spied from page start). */
const packetsReceived = (page: Page): Promise<number> =>
  page.evaluate(async () => {
    let total = 0;
    for (const pc of (window as unknown as { __pcs: RTCPeerConnection[] }).__pcs) {
      (await pc.getStats()).forEach((r: { type: string; kind?: string; packetsReceived?: number }) => {
        if (r.type === "inbound-rtp" && r.kind === "audio") total += r.packetsReceived ?? 0;
      });
    }
    return total;
  });

test("web host → web listener: media, shared transport, playhead follows play/stop/seek", async ({ browser }) => {
  test.setTimeout(240_000);
  const errors: string[] = [];
  // The transport bar is tight at 1280 px (see collab-host.spec.ts).
  const viewport = { width: 1600, height: 900 };
  const ctxA = await browser.newContext({ viewport });
  const ctxB = await browser.newContext({ viewport });
  // Spy on Bob's peer connections (to check that audio really arrives).
  await ctxB.addInitScript(() => {
    const w = window as unknown as { __pcs: RTCPeerConnection[]; RTCPeerConnection: typeof RTCPeerConnection };
    w.__pcs = [];
    const Native = w.RTCPeerConnection;
    w.RTCPeerConnection = class extends Native {
      constructor(config?: RTCConfiguration) {
        super(config);
        w.__pcs.push(this);
      }
    };
  });
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));
  await open(a);
  await open(b);
  await join(a, "Ada");
  await join(b, "Bob");

  // --- Bob listens on Ada's computer (enabled once her presence says she can host).
  const chip = b.locator('[data-testid="collab-peers"] [data-peer="Ada"]');
  await expect(chip).toBeVisible({ timeout: 20_000 });
  const entry = b.getByRole("menuitem", { name: "Listen on Ada's computer" });
  await expect
    .poll(
      async () => {
        await chip.click({ button: "right" });
        const enabled = await entry.isEnabled().catch(() => false);
        if (!enabled) await b.keyboard.press("Escape");
        return enabled;
      },
      { timeout: 20_000 },
    )
    .toBe(true);
  await entry.click();
  const status = b.getByTestId("listen-status");
  await expect(status).toHaveAttribute("aria-label", "Listening to Ada", { timeout: 30_000 });
  await expect.poll(() => packetsReceived(b), { timeout: 20_000 }).toBeGreaterThan(10);

  // --- Ada plays: Bob's transport follows and his playhead moves with what he hears.
  await playButton(a).click();
  await expect.poll(async () => (await transportState(b))?.playing, { timeout: 10_000 }).toBe(true);
  await expect.poll(() => bars(b), { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
  expect(await playButton(b).count()).toBe(0); // Bob's bar shows the host playing (Stop)

  // --- Bob's Stop stops Ada (a TransportRequest); the stop anchor freezes his playhead.
  const bobStop = b.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true }).first();
  await bobStop.click();
  await expect(playButton(a)).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => (await transportState(b))?.playing, { timeout: 10_000 }).toBe(false);
  // Once the stop anchor's sample is heard, Bob's playhead rests where Ada stopped.
  const stoppedAt = await a.getByTestId("position-bars").textContent();
  await expect.poll(() => b.getByTestId("position-bars").textContent(), { timeout: 10_000 }).toBe(stoppedAt);

  // --- A second Stop returns Ada to the start: Bob's playhead follows the seek anchor.
  await bobStop.click();
  await expect(a.getByTestId("position-bars")).toHaveText("1.1.1", { timeout: 10_000 });
  await expect(b.getByTestId("position-bars")).toHaveText("1.1.1", { timeout: 10_000 });

  // --- Ada sees Bob listening; Bob stops listening and Ada's list empties.
  await expect(a.locator('[data-testid="collab-peers"] [data-peer="Bob"] [data-testid="listening-badge"]')).toBeVisible({ timeout: 10_000 });
  await status.getByRole("button", { name: "Stop listening" }).click();
  await expect(status).toHaveCount(0);
  await expect(a.locator('[data-testid="collab-peers"] [data-peer="Bob"] [data-testid="listening-badge"]')).toHaveCount(0, { timeout: 10_000 });
  await expect(playButton(b)).toBeVisible();

  await ctxA.close();
  await ctxB.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
