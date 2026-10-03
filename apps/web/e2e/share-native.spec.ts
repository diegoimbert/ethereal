// Sharing between a native engine and a browser (docs/SHARING.md §12, node
// `share-integration`), on loopback through the Node signaling adapter:
//
// - the native engine is `ether-server` (the desktop app's engine, headless, null audio),
//   driven over its WebSocket protocol by this test. Its sharing services are the real
//   defaults: tungstenite signaling and str0m data channels (`default_services()`);
// - the browser is the web app with its in-browser engine (hub in the controller Worker,
//   `RTCPeerConnection`s in the page).
//
// 1. native host → web joiner: the server shares; the browser opens the link, joins, and
//    edits travel both ways; the server stops sharing and the browser is told.
// 2. web host → native joiner: the browser shares; the server opens the link and joins
//    (`OpenInvite` → `Ready` → `AcceptInvite`); edits travel both ways; the server leaves.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Browser, type Page } from "@playwright/test";
import type { Project, ShareState } from "@/generated";
import { createTrack, playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;

test.use({
  viewport: { width: 1440, height: 900 },
  launchOptions: {
    args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--disable-features=WebRtcHideLocalIpsWithMdns"],
  },
});

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);

const names = (p: Project | null | undefined): string[] => (p ? Object.values(p.tracks).map((t) => t.name).sort() : []);

/** A 26-char ULID (Crockford base32): the id format of tracks. */
function ulid(): string {
  const A = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let t = Date.now();
  let time = "";
  for (let i = 0; i < 10; i++) {
    time = A[t % 32]! + time;
    t = Math.floor(t / 32);
  }
  let rand = "";
  for (let i = 0; i < 16; i++) rand += A[Math.floor(Math.random() * 32)]!;
  return time + rand;
}

/** A UUIDv7: the id format of projects. */
function uuidv7(): string {
  const b = new Uint8Array(16);
  crypto.getRandomValues(b);
  let ts = Date.now();
  for (let i = 5; i >= 0; i--) {
    b[i] = ts % 256;
    ts = Math.floor(ts / 256);
  }
  b[6] = 0x70 | (b[6]! & 0x0f);
  b[8] = 0x80 | (b[8]! & 0x3f);
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

// --- processes ------------------------------------------------------------------------------

function buildServer(): string {
  const out = execFileSync("cargo", ["build", "-p", "ether-server", "--bin", "ether-server", "--message-format=json"], {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-server" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the ether-server binary");
}

/** The Node signaling adapter (as in share.spec.ts; Node needs `--experimental-transform-types`). */
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

let signal: ChildProcess | null = null;
let signalUrl = "";
let serverBin = "";
const servers: { child: ChildProcess; dir: string }[] = [];

test.beforeAll(async () => {
  test.setTimeout(900_000);
  serverBin = buildServer();
  ({ url: signalUrl, child: signal } = await startSignal());
});

test.afterAll(() => {
  signal?.kill();
  for (const s of servers) {
    s.child.kill();
    rmSync(s.dir, { recursive: true, force: true });
  }
});

/** A fresh `ether-server` (own data dir): its WebSocket URL. */
async function startServer(): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), "share-integration-native-"));
  const child = spawn(serverBin, ["--port", "0", "--token", TOKEN, "--name", "studio", "--data-dir", dir, "--library", join(dir, "library")], {
    env: { ...process.env, ETHER_AUDIO: "null", RUST_LOG: "warn" },
    stdio: ["ignore", "pipe", "inherit"],
  });
  servers.push({ child, dir });
  return new Promise<string>((ok, fail) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) ok(m[1]!);
    });
    child.on("exit", (code) => fail(new Error(`ether-server exited (${code})`)));
  });
}

/** The native engine, driven over the remote protocol: commands, and the last `ShareState`. */
class Native {
  private ws!: WebSocket;
  private next = 1;
  private readonly waiting = new Map<number, (r: { status: string; value?: unknown; error?: unknown }) => void>();
  share: ShareState = { type: "Off" };
  notices: string[] = [];

  async open(url: string): Promise<void> {
    this.ws = new WebSocket(url);
    this.ws.binaryType = "arraybuffer";
    await new Promise((ok, fail) => {
      this.ws.onopen = ok;
      this.ws.onerror = fail;
    });
    this.ws.send(JSON.stringify({ protocol_version: 1, token: TOKEN, client: "share e2e" }));
    await new Promise<void>((ok) => {
      this.ws.onmessage = (hello) => {
        expect(JSON.parse(String(hello.data)).type).toBe("Welcome");
        this.ws.onmessage = (e) => {
          if (typeof e.data !== "string") return;
          const m = JSON.parse(e.data) as { kind: string; body: Record<string, unknown> };
          if (m.kind === "Reply") {
            const b = m.body as { id: number; result: { status: string } };
            this.waiting.get(b.id)?.(b.result);
          } else if (m.kind === "Event" && m.body.type === "Share") {
            const ev = m.body.event as { type: string; state?: ShareState; notice?: { type: string } };
            if (ev.type === "State" && ev.state) this.share = ev.state;
            if (ev.type === "Notice" && ev.notice) this.notices.push(ev.notice.type);
          }
        };
        ok();
      };
    });
  }

  async send<T = unknown>(domain: string, command: object): Promise<T> {
    const id = this.next++;
    const r = await new Promise<{ status: string; value?: unknown; error?: unknown }>((ok) => {
      this.waiting.set(id, ok);
      this.ws.send(JSON.stringify({ id, gesture: null, command: { domain, command } }));
    });
    if (r.status !== "Ok") throw new Error(`${domain} ${JSON.stringify(command)} failed: ${JSON.stringify(r.error)}`);
    return r.value as T;
  }

  async project(): Promise<Project> {
    return (await this.send<{ project: Project }>("Project", { type: "Get" })).project;
  }

  async addTrack(name: string): Promise<void> {
    await this.send("Track", { type: "Create", id: ulid(), kind: "Midi", name, color: null, parent: null, before: null });
  }

  close() {
    this.ws.close();
  }
}

/** A native engine with an identity, pointed at the test's signaling service. */
async function nativeEngine(name: string): Promise<Native> {
  const n = new Native();
  await n.open(await startServer());
  await n.send("Share", { type: "SetIdentity", name, color: null });
  await n.send("Share", { type: "SetServers", signal_url: signalUrl, invite_origin: null });
  return n;
}

async function browserPerson(browser: Browser, name: string): Promise<Page> {
  const ctx = await browser.newContext({ permissions: ["clipboard-read", "clipboard-write"] });
  await ctx.addInitScript(
    ([n, s]) => {
      localStorage.setItem("eth.share.identity", JSON.stringify({ name: n, color: null }));
      localStorage.setItem("eth-share-settings", JSON.stringify({ signalUrl: s }));
    },
    [name, signalUrl] as const,
  );
  const page = await ctx.newPage();
  return page;
}

async function boot(page: Page): Promise<void> {
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

/** The link as this test's web server serves it. */
function local(link: string, page: Page): string {
  const u = new URL(link);
  return `${new URL(page.url()).origin}${u.pathname}${u.search}${u.hash}`;
}

test("native host (ether-server) → web joiner", async ({ browser }) => {
  test.setTimeout(180_000);
  const studio = await nativeEngine("Studio");
  const pid = uuidv7();
  await studio.send("Project", { type: "Create", id: pid, name: "Native song" });
  await studio.addTrack("Native drums");
  await studio.send("Share", { type: "Start" });
  await expect.poll(() => studio.share.type === "Hosting" && studio.share.signal.type, { timeout: 20_000 }).toBe("Online");
  const link = studio.share.type === "Hosting" ? studio.share.edit_link! : "";
  expect(link).toContain(`?s=${encodeURIComponent(signalUrl)}`);

  const ada = await browserPerson(browser, "Ada");
  const errors: string[] = [];
  ada.on("pageerror", (e) => errors.push(e.message));
  await ada.goto("/");
  await ada.goto(local(link, ada));
  await ada.getByTestId("join-landing").getByRole("button", { name: "Continue in browser" }).click();
  await expect(ada.getByTestId("join-ready")).toContainText("Studio invites you to Native song", { timeout: 30_000 });
  await ada.getByTestId("join-accept").click();
  await expect(ada.getByText("You're in Native song with Studio")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(ada).then((p) => p?.id)).toBe(pid);
  await expect.poll(() => project(ada).then(names)).toContain("Native drums");
  await expect.poll(() => studio.notices).toContain("ParticipantJoined");

  // Edits both ways.
  await createTrack(ada, "Audio");
  await expect.poll(async () => names(await studio.project()), { timeout: 10_000 }).toEqual(names(await project(ada)));
  await studio.addTrack("Native bass");
  await expect.poll(() => project(ada).then(names), { timeout: 10_000 }).toContain("Native bass");
  expect(names(await project(ada))).toEqual(names(await studio.project()));

  // The native host stops sharing: the browser keeps its copy.
  await studio.send("Share", { type: "Stop" });
  await expect(ada.locator(".eth-toast").filter({ hasText: "stopped sharing" })).toBeVisible({ timeout: 30_000 });
  await expect(ada.locator('[data-feature="share"]')).toHaveAttribute("data-state", "Off");
  expect(names(await project(ada))).toContain("Native bass");
  studio.close();
  expect(errors).toEqual([]);
});

test("web host → native joiner (ether-server)", async ({ browser }) => {
  test.setTimeout(180_000);
  const diego = await browserPerson(browser, "Diego");
  const errors: string[] = [];
  diego.on("pageerror", (e) => errors.push(e.message));
  await boot(diego);
  await createTrack(diego, "Midi");
  await diego.getByTestId("share-button").click();
  const pop = diego.getByTestId("share-popover");
  await expect(pop.getByLabel("Edit link")).toHaveValue(/\/join\//, { timeout: 20_000 });
  const link = await pop.getByLabel("Edit link").inputValue();
  await diego.keyboard.press("Escape");

  const studio = await nativeEngine("Studio");
  await studio.send("Share", { type: "OpenInvite", link });
  await expect
    .poll(() => (studio.share.type === "Joining" ? studio.share.stage.type : studio.share.type), { timeout: 30_000 })
    .toBe("Ready");
  if (studio.share.type === "Joining" && studio.share.stage.type === "Ready") {
    expect(studio.share.stage.invite.host.name).toBe("Diego");
    expect(studio.share.stage.invite.role).toBe("Edit");
  }
  await studio.send("Share", { type: "AcceptInvite" });
  await expect
    .poll(() => (studio.share.type === "Joined" ? studio.share.link.type : studio.share.type), { timeout: 30_000 })
    .toBe("Online");
  const pid = (await project(diego))!.id;
  await expect.poll(async () => (await studio.project()).id, { timeout: 30_000 }).toBe(pid);
  await expect.poll(async () => names(await studio.project())).toEqual(names(await project(diego)));
  await expect(diego.locator(".eth-toast").filter({ hasText: "Studio joined" })).toBeVisible({ timeout: 10_000 });

  // Edits both ways.
  await studio.addTrack("From the studio");
  await expect.poll(() => project(diego).then(names), { timeout: 10_000 }).toContain("From the studio");
  await createTrack(diego, "Audio");
  await expect.poll(async () => names(await studio.project()), { timeout: 10_000 }).toEqual(names(await project(diego)));

  // The native joiner leaves: the host shows it offline.
  await studio.send("Share", { type: "Leave" });
  await expect(diego.locator(".eth-toast").filter({ hasText: "Studio left" })).toBeVisible({ timeout: 10_000 });
  studio.close();
  expect(errors).toEqual([]);
});
