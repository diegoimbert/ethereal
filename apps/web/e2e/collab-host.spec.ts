// "Listen on <peer>", host side on the web build (docs/COLLAB.md §9, node `stream-host`):
// two browser contexts join one session on a local `ether-collab-relay`.
//
// - The worklet has the stream tap output (output 1) and the transport exposes it as a
//   MediaStream; the tap carries the metronome while playing (master + metronome).
// - A declares `SetHosting { ui_sender: true }` on join (Chromium has WebRTC + encoded
//   transforms), so B sees A's presence with `can_host`; A's hosting toggle turns it off and
//   on again.
// - The listen round trip (B listens, A's web sender streams, B's playhead follows) needs the
//   receiver (`stream-listen`): `test.fixme` until it lands.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Presence, Project } from "@/generated";
import { playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-host-${Math.random().toString(36).slice(2, 8)}`;

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);

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
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
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

/**
 * Record the peers' presence as the controller Worker reports it (`Event::Collab
 * Presence`), in `window.__peers`: the UI does not show `can_host` yet (stream-listen's
 * "Listen" menu will).
 */
async function recordPresence(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as {
      __peers: Presence[];
      __etherEngine: { handles(): { controller: Worker } | null };
    };
    w.__peers = [];
    w.__etherEngine.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type !== "server" || !e.data.json) return;
      for (const m of JSON.parse(e.data.json) as { kind: string; body: { type?: string; event?: { type: string; peers?: Presence[] } } }[]) {
        if (m.kind === "Event" && m.body.type === "Collab" && m.body.event?.type === "Presence") w.__peers = m.body.event.peers!;
      }
    });
  });
}

const canHost = (page: Page, name: string): Promise<boolean | null> =>
  page.evaluate((n) => {
    const p = (window as unknown as { __peers: Presence[] }).__peers.find((x) => x.name === n);
    return p ? (p.state.can_host ?? false) : null;
  }, name);

test("web host: stream tap output, SetHosting and the hosting toggle", async ({ browser }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  // Same workaround as tempo-metronome.spec.ts: at 1280 px the transport bar's right side
  // overlaps the Metronome button.
  const viewport = { width: 1600, height: 900 };
  const ctxA = await browser.newContext({ viewport });
  const ctxB = await browser.newContext({ viewport });
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));
  await open(a);
  await open(b);

  // --- The stream tap: worklet output 1 → MediaStream; it carries the metronome.
  const tap = await a.evaluate(() => {
    const e = (window as unknown as {
      __etherEngine: {
        handles(): { node: AudioWorkletNode; context: AudioContext } | null;
        streamOutput(): { stream: MediaStream } | null;
      };
    }).__etherEngine;
    const h = e.handles()!;
    const out = e.streamOutput()!;
    const analyser = h.context.createAnalyser();
    analyser.fftSize = 2048;
    h.node.connect(analyser, 1);
    (window as unknown as { __tapAnalyser: AnalyserNode }).__tapAnalyser = analyser;
    return { outputs: h.node.numberOfOutputs, tracks: out.stream.getAudioTracks().length };
  });
  expect(tap).toEqual({ outputs: 2, tracks: 1 });
  // Peak of the tap over ~1.2 s (more than two beats at 120 bpm: a click is short).
  const tapPeak = () =>
    a.evaluate(
      () =>
        new Promise<number>((done) => {
          const an = (window as unknown as { __tapAnalyser: AnalyserNode }).__tapAnalyser;
          const buf = new Float32Array(an.fftSize);
          let peak = 0;
          const started = performance.now();
          const timer = setInterval(() => {
            an.getFloatTimeDomainData(buf);
            for (const v of buf) peak = Math.max(peak, Math.abs(v));
            if (performance.now() - started > 1_200) {
              clearInterval(timer);
              done(peak);
            }
          }, 20);
        }),
    );
  await a.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Metronome" }).click();
  await playButton(a).click();
  await expect.poll(tapPeak, { timeout: 10_000 }).toBeGreaterThan(0.01);
  await a.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true }).first().click();

  // --- A hosts from its UI: B sees `can_host`.
  await recordPresence(b);
  await join(a, "Ada");
  await join(b, "Bob");
  await expect.poll(() => canHost(b, "Ada"), { timeout: 20_000 }).toBe(true);

  // --- The hosting toggle (collab dialog).
  await a.getByTestId("collab-button").click();
  const allow = a.getByRole("switch", { name: "Let others listen to my computer" });
  await expect(allow).toHaveAttribute("aria-checked", "true");
  await expect(a.getByRole("note")).toHaveCount(0); // this browser can stream
  await allow.click();
  await expect(allow).toHaveAttribute("aria-checked", "false");
  await expect.poll(() => canHost(b, "Ada"), { timeout: 10_000 }).toBe(false);
  await allow.click();
  await expect.poll(() => canHost(b, "Ada"), { timeout: 10_000 }).toBe(true);
  await a.getByRole("button", { name: "Close" }).click();

  await ctxA.close();
  await ctxB.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

// Needs stream-listen (the receiver UI + the controller's `Listen`): B listens on A, A's web
// sender opens a peer connection (offer with the Opus parameters, ICE through the relay),
// media flows, B receives anchors (`StreamClock`) and its playhead follows A's play/stop/loop
// wrap; A's listener list shows Bob; B stops listening and A's connection closes.
test.fixme("web host → web listener round trip (needs stream-listen)", async () => {});
