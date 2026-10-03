// The web share endpoint end to end (docs/SHARING.md §6.2, node `p2p-transport`): two
// browser contexts, each with its controller Worker's `WebPeers` (`ShareProbe`, crates/
// ether-wasm/src/share.rs) driving the UI thread's `RTCPeerConnection` over the share port.
// The test plays the signaling service (it passes each side's signals to the other), then
// one side echoes every frame it receives: text and a 1 MB binary frame (cut into 16 KiB
// data-channel messages in the Worker, reassembled on the other Worker) come back whole and
// in order.
import { expect, test, type Page } from "@playwright/test";

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
  await page.goto("/");
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

  await probe(host, "open", PEER, false, "[]");
  await probe(joiner, "open", PEER, true, "[]");

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
