// The web share endpoint end to end (docs/SHARING.md §6, node `p2p-transport`): the
// controller Worker's `WebPeers` (`ShareProbe`, crates/ether-wasm/src/share.rs) drives the UI
// thread's `RTCPeerConnection` over the share port. The test plays the signaling service (it
// passes each side's signals to the other), then one side echoes every frame it receives:
// text and a 1 MB binary frame (cut into 16 KiB data-channel messages, reassembled on the
// other side) come back whole and in order.
// - two browser contexts;
// - a browser and the native str0m endpoint (`crates/ether-collab/tests/p2p_stdio.rs`, driven
//   over stdin/stdout), both ways round (web joiner + native host, native joiner + web host).
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { launch } from "./ui";

// Real host candidates (no mDNS `.local` names: the devbox may not resolve them).
test.use({
  launchOptions: {
    args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--disable-features=WebRtcHideLocalIpsWithMdns"],
  },
});

type ProbeOutput =
  | { type: "Signal"; peer: number; signal: unknown }
  | { type: "Connected"; peer: number; local: string; remote: string }
  | { type: "Failed"; peer: number; reason: string };
type ProbeFrame = { type: "Text"; text: string } | { type: "Binary"; len: number; sum: number };

const PEER = 7;

function probe<T>(page: Page, method: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(
    ([m, a]) =>
      (
        window as unknown as { __etherEngine: { shareProbe(method: string, ...args: unknown[]): Promise<unknown> } }
      ).__etherEngine.shareProbe(m as string, ...(a as unknown[])),
    [method, args] as const,
  ) as Promise<T>;
}

async function open(page: Page): Promise<void> {
  await launch(page);
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __etherEngine: { handles(): unknown } }).__etherEngine.handles() !== null))
    .toBe(true);
}

/** Byte `i` of `send_pattern(len, seed)`, summed (u32 wrapping). */
function patternSum(len: number, seed: number): number {
  let sum = 0;
  for (let i = 0; i < len; i++) sum = (sum + ((((i & 255) * 31) & 255) + seed) % 256) >>> 0;
  return sum;
}

test("web data channel echo through the share port (two contexts)", async ({ browser }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  const host = await (await browser.newContext()).newPage();
  const joiner = await (await browser.newContext()).newPage();
  for (const p of [host, joiner]) p.on("pageerror", (e) => errors.push(e.message));
  await open(host);
  await open(joiner);

  await probe(host, "open", PEER, false, "[]", false);
  await probe(joiner, "open", PEER, true, "[]", false);

  // Signaling by hand until both data channels are open.
  const connected = new Map<Page, { local: string; remote: string }>();
  const deadline = Date.now() + 30_000;
  while (connected.size < 2) {
    expect(Date.now(), "the data channel never opened").toBeLessThan(deadline);
    for (const [from, to] of [
      [joiner, host],
      [host, joiner],
    ] as const) {
      const outputs = JSON.parse(await probe<string>(from, "poll")) as ProbeOutput[];
      for (const o of outputs) {
        expect(o.peer).toBe(PEER);
        if (o.type === "Failed") throw new Error(`pairing failed: ${o.reason}`);
        if (o.type === "Signal") await probe(to, "signal", PEER, JSON.stringify(o.signal));
        if (o.type === "Connected") connected.set(from, { local: o.local, remote: o.remote });
      }
    }
    await host.waitForTimeout(20);
  }
  const h = connected.get(host)!;
  const j = connected.get(joiner)!;
  expect(h.local).toBe(j.remote);
  expect(h.remote).toBe(j.local);
  expect(h.local).toMatch(/^sha-256 ([0-9A-F]{2}:){31}[0-9A-F]{2}$/);
  expect(await probe(host, "state", PEER)).toBe("open");

  // The joiner sends; the host echoes everything back.
  const texts = Array.from({ length: 20 }, (_, i) => `frame ${i} ${"é".repeat(i * 100)}`);
  await probe(joiner, "send_text", PEER, "hello");
  await probe(joiner, "send_pattern", PEER, 1_000_000, 5);
  for (const t of texts) await probe(joiner, "send_text", PEER, t);
  const expected: ProbeFrame[] = [
    { type: "Text", text: "hello" },
    { type: "Binary", len: 1_000_000, sum: patternSum(1_000_000, 5) },
    ...texts.map((text) => ({ type: "Text" as const, text })),
  ];

  const atHost: ProbeFrame[] = [];
  const echoed: ProbeFrame[] = [];
  const end = Date.now() + 30_000;
  while (echoed.length < expected.length) {
    expect(Date.now(), `echo stalled at ${echoed.length}/${expected.length}`).toBeLessThan(end);
    atHost.push(...(JSON.parse(await probe<string>(host, "recv", PEER, true)) as ProbeFrame[]));
    echoed.push(...(JSON.parse(await probe<string>(joiner, "recv", PEER, false)) as ProbeFrame[]));
    await host.waitForTimeout(20);
  }
  expect(atHost).toEqual(expected);
  expect(echoed).toEqual(expected);
  await expect.poll(() => probe<number>(joiner, "buffered", PEER)).toBe(0);

  // Closing one side closes the other's link.
  await probe(joiner, "close", PEER);
  await expect.poll(() => probe<string>(host, "state", PEER), { timeout: 20_000 }).toMatch(/^closed/);
  expect(errors).toEqual([]);
});

// ─── native ↔ browser ────────────────────────────────────────────────────────────────────

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
let nativeBin = "";

/** Build the stdio test binary and return its path (cargo tells where it is). */
function buildNativePeer(): string {
  const out = execFileSync("cargo", ["test", "-p", "ether-collab", "--test", "p2p_stdio", "--no-run", "--message-format=json"], {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "p2p_stdio" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the p2p_stdio test binary");
}

type NativeEvent =
  | { type: "Signal"; signal: unknown }
  | { type: "Connected"; local: string; remote: string }
  | { type: "Failed"; reason: string }
  | { type: "Frame"; text?: string; len?: number; sum?: number }
  | { type: "Closed"; reason: string };

class NativePeer {
  readonly events: NativeEvent[] = [];
  private read = 0;
  private readonly child: ChildProcess;

  constructor(offer: boolean) {
    this.child = spawn(nativeBin, ["--ignored", "--nocapture", "--exact", "stdio_peer"], {
      env: { ...process.env, P2P_STDIO_OFFER: offer ? "1" : "0", RUST_LOG: "warn" },
      stdio: ["pipe", "pipe", "inherit"],
    });
    // The peer exits on its own once its link closes: later writes are dropped.
    this.child.stdin!.on("error", () => undefined);
    createInterface({ input: this.child.stdout! }).on("line", (line) => {
      if (line.startsWith("P2P ")) this.events.push(JSON.parse(line.slice(4)) as NativeEvent);
    });
  }
  /** Events since the last call. */
  take(): NativeEvent[] {
    const out = this.events.slice(this.read);
    this.read = this.events.length;
    return out;
  }
  signal(signal: unknown) {
    this.child.stdin!.write(`${JSON.stringify({ type: "Signal", signal })}\n`);
  }
  quit() {
    if (this.child.exitCode === null && !this.child.stdin!.writableEnded) this.child.stdin!.end(`${JSON.stringify({ type: "Quit" })}\n`);
  }
}

test.describe("native ↔ browser", () => {
  test.beforeAll(() => {
    test.setTimeout(600_000);
    nativeBin = buildNativePeer();
  });

  for (const nativeHosts of [true, false]) {
    test(`data channel echo: ${nativeHosts ? "web joiner, native host" : "native joiner, web host"}`, async ({ browser }) => {
      test.setTimeout(120_000);
      const page = await (await browser.newContext()).newPage();
      const errors: string[] = [];
      page.on("pageerror", (e) => errors.push(e.message));
      await open(page);
      const native = new NativePeer(!nativeHosts);
      try {
        await probe(page, "open", PEER, nativeHosts, "[]", false);
        let web: { local: string; remote: string } | null = null;
        let nat: { local: string; remote: string } | null = null;
        const deadline = Date.now() + 40_000;
        while (!web || !nat) {
          expect(Date.now(), "the data channel never opened").toBeLessThan(deadline);
          for (const o of JSON.parse(await probe<string>(page, "poll")) as ProbeOutput[]) {
            if (o.type === "Failed") throw new Error(`web pairing failed: ${o.reason}`);
            if (o.type === "Signal") native.signal(o.signal);
            if (o.type === "Connected") web = { local: o.local, remote: o.remote };
          }
          for (const e of native.take()) {
            if (e.type === "Failed") throw new Error(`native pairing failed: ${e.reason}`);
            if (e.type === "Signal") await probe(page, "signal", PEER, JSON.stringify(e.signal));
            if (e.type === "Connected") nat = { local: e.local, remote: e.remote };
          }
          await page.waitForTimeout(20);
        }
        // Both sides see the same two fingerprints, in the same normalized form.
        expect(web.local).toBe(nat.remote);
        expect(web.remote).toBe(nat.local);

        const texts = Array.from({ length: 10 }, (_, i) => `native ${i} ${"ü".repeat(i * 500)}`);
        await probe(page, "send_text", PEER, "hello");
        await probe(page, "send_pattern", PEER, 1_000_000, 9);
        for (const t of texts) await probe(page, "send_text", PEER, t);
        const expected: ProbeFrame[] = [
          { type: "Text", text: "hello" },
          { type: "Binary", len: 1_000_000, sum: patternSum(1_000_000, 9) },
          ...texts.map((text) => ({ type: "Text" as const, text })),
        ];
        const echoed: ProbeFrame[] = [];
        const end = Date.now() + 30_000;
        while (echoed.length < expected.length) {
          expect(Date.now(), `echo stalled at ${echoed.length}/${expected.length}`).toBeLessThan(end);
          echoed.push(...(JSON.parse(await probe<string>(page, "recv", PEER, false)) as ProbeFrame[]));
          await page.waitForTimeout(20);
        }
        expect(echoed).toEqual(expected);
        const atNative = native.events.filter((e) => e.type === "Frame");
        expect(atNative).toEqual(expected.map((f) => (f.type === "Text" ? { type: "Frame", text: f.text } : { type: "Frame", len: f.len, sum: f.sum })));

        // The web side closes: the native link closes too.
        await probe(page, "close", PEER);
        await expect.poll(() => native.events.some((e) => e.type === "Closed"), { timeout: 20_000 }).toBe(true);
        expect(errors).toEqual([]);
      } finally {
        native.quit();
      }
    });
  }
});
