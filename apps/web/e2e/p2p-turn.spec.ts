// Native ↔ browser data channels through TURN only (docs/SHARING.md §6.1, node
// `native-turn`). A loopback TURN server (the relay's experimental one,
// `crates/ether-collab/tests/p2p_turn.rs` `turn_server_stdio`) relays between the browser
// (`iceTransportPolicy: "relay"`, the share port's `relay` flag) and the native str0m
// endpoint with "Hide my IP" on (`p2p_stdio.rs` with `P2P_STDIO_RELAY=1`: no host
// candidates, its own TURN client). Every candidate either side sends is a relay
// candidate, so the pairing only works through the relay. Both ways round, then a text and
// a 200 kB binary frame are echoed whole and in order.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";

test.use({
  launchOptions: { args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio"] },
});

type ProbeOutput =
  | { type: "Signal"; peer: number; signal: Signal }
  | { type: "Connected"; peer: number; local: string; remote: string }
  | { type: "Failed"; peer: number; reason: string };
type ProbeFrame = { type: "Text"; text: string } | { type: "Binary"; len: number; sum: number };
type Signal = { type: string; sdp?: string; candidate?: { candidate: string } };
type IceServer = { urls: string[]; username: string | null; credential: string | null };

const PEER = 11;
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

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
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __etherEngine: { handles(): unknown } }).__etherEngine.handles() !== null))
    .toBe(true);
}

function patternSum(len: number, seed: number): number {
  let sum = 0;
  for (let i = 0; i < len; i++) sum = (sum + ((((i & 255) * 31) & 255) + seed) % 256) >>> 0;
  return sum;
}

/** Build a cargo test binary and return its path. */
function buildTest(name: string, features: string[] = []): string {
  const args = ["test", "-p", "ether-collab", "--test", name, "--no-run", "--message-format=json"];
  if (features.length) args.push("--features", features.join(","));
  const out = execFileSync("cargo", args, {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === name && msg.executable) return msg.executable;
  }
  throw new Error(`cargo did not report the ${name} test binary`);
}

/** The loopback TURN server; resolves with its ICE server entry. */
function startTurn(bin: string): Promise<{ child: ChildProcess; ice: IceServer }> {
  const child = spawn(bin, ["--ignored", "--nocapture", "--exact", "turn_server_stdio"], {
    env: { ...process.env, RUST_LOG: "error" },
    stdio: ["pipe", "pipe", "inherit"],
  });
  return new Promise((ok, fail) => {
    const timer = setTimeout(() => fail(new Error("the TURN server did not start")), 30_000);
    createInterface({ input: child.stdout! }).on("line", (line) => {
      if (!line.startsWith("TURN ")) return;
      clearTimeout(timer);
      ok({ child, ice: JSON.parse(line.slice(5)) as IceServer });
    });
    child.on("exit", (code) => fail(new Error(`the TURN server exited (${code})`)));
  });
}

type NativeEvent =
  | { type: "Signal"; signal: Signal }
  | { type: "Connected"; local: string; remote: string }
  | { type: "Failed"; reason: string }
  | { type: "Frame"; text?: string; len?: number; sum?: number }
  | { type: "Closed"; reason: string };

class NativePeer {
  readonly events: NativeEvent[] = [];
  private read = 0;
  private readonly child: ChildProcess;

  constructor(bin: string, offer: boolean, ice: IceServer[]) {
    this.child = spawn(bin, ["--ignored", "--nocapture", "--exact", "stdio_peer"], {
      env: { ...process.env, P2P_STDIO_OFFER: offer ? "1" : "0", P2P_STDIO_ICE: JSON.stringify(ice), P2P_STDIO_RELAY: "1", RUST_LOG: "warn" },
      stdio: ["pipe", "pipe", "inherit"],
    });
    this.child.stdin!.on("error", () => undefined);
    createInterface({ input: this.child.stdout! }).on("line", (line) => {
      if (line.startsWith("P2P ")) this.events.push(JSON.parse(line.slice(4)) as NativeEvent);
    });
  }
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

/** The ICE candidates in a signal (SDP lines or a trickled one). */
function candidatesOf(s: Signal): string[] {
  if (s.type === "Ice") return s.candidate?.candidate ? [s.candidate.candidate] : [];
  return (s.sdp ?? "").split(/\r?\n/).filter((l) => l.startsWith("a=candidate:"));
}

test.describe("native ↔ browser through TURN (relay only)", () => {
  let nativeBin = "";
  let turn: { child: ChildProcess; ice: IceServer } | null = null;

  test.beforeAll(async () => {
    test.setTimeout(900_000);
    nativeBin = buildTest("p2p_stdio");
    turn = await startTurn(buildTest("p2p_turn", ["turn"]));
  });
  test.afterAll(() => {
    turn?.child.stdin?.end();
    turn?.child.kill();
  });

  for (const nativeHosts of [true, false]) {
    test(`relayed echo: ${nativeHosts ? "web joiner, native host" : "native joiner, web host"}`, async ({ browser }) => {
      test.setTimeout(120_000);
      const ice = [turn!.ice];
      const page = await (await browser.newContext()).newPage();
      const errors: string[] = [];
      page.on("pageerror", (e) => errors.push(e.message));
      await open(page);
      const native = new NativePeer(nativeBin, !nativeHosts, ice);
      const seen: string[] = [];
      try {
        await probe(page, "open", PEER, nativeHosts, JSON.stringify(ice), true);
        let web: { local: string; remote: string } | null = null;
        let nat: { local: string; remote: string } | null = null;
        const deadline = Date.now() + 40_000;
        while (!web || !nat) {
          expect(Date.now(), "the data channel never opened").toBeLessThan(deadline);
          for (const o of JSON.parse(await probe<string>(page, "poll")) as ProbeOutput[]) {
            if (o.type === "Failed") throw new Error(`web pairing failed: ${o.reason}`);
            if (o.type === "Signal") {
              seen.push(...candidatesOf(o.signal));
              native.signal(o.signal);
            }
            if (o.type === "Connected") web = { local: o.local, remote: o.remote };
          }
          for (const e of native.take()) {
            if (e.type === "Failed") throw new Error(`native pairing failed: ${e.reason}`);
            if (e.type === "Signal") {
              seen.push(...candidatesOf(e.signal));
              await probe(page, "signal", PEER, JSON.stringify(e.signal));
            }
            if (e.type === "Connected") nat = { local: e.local, remote: e.remote };
          }
          await page.waitForTimeout(20);
        }
        expect(web.local).toBe(nat.remote);
        expect(web.remote).toBe(nat.local);
        // Relay candidates only, from both sides.
        expect(seen.length).toBeGreaterThanOrEqual(2);
        for (const c of seen) expect(c).toContain(" typ relay");

        await probe(page, "send_text", PEER, "through the relay");
        await probe(page, "send_pattern", PEER, 200_000, 3);
        const expected: ProbeFrame[] = [
          { type: "Text", text: "through the relay" },
          { type: "Binary", len: 200_000, sum: patternSum(200_000, 3) },
        ];
        const echoed: ProbeFrame[] = [];
        // The test TURN server relays 64 KB/s per allocation (COLLAB.md §10).
        const end = Date.now() + 60_000;
        while (echoed.length < expected.length) {
          expect(Date.now(), `echo stalled at ${echoed.length}/${expected.length}`).toBeLessThan(end);
          echoed.push(...(JSON.parse(await probe<string>(page, "recv", PEER, false)) as ProbeFrame[]));
          await page.waitForTimeout(20);
        }
        expect(echoed).toEqual(expected);

        await probe(page, "close", PEER);
        await expect.poll(() => native.events.some((e) => e.type === "Closed"), { timeout: 20_000 }).toBe(true);
        expect(errors).toEqual([]);
      } finally {
        native.quit();
      }
    });
  }
});
