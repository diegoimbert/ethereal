// Listen on a peer, listener side (docs/COLLAB.md §9; node `stream-listen`).
//
// A real listener (the web app, its engine in the browser) and a **scripted host**: a
// second browser context that joins the session over the relay's WebSocket protocol by
// hand and answers like a web host would (an oscillator over a real RTCPeerConnection,
// stream clock anchors from the RTP timestamps it observes through encoded streams). Once
// `stream-host` lands, the two-app variant replaces the scripted host.
//
// start the relay → Ada joins → the scripted host "Hal" joins with can_host → Ada listens on
// Hal's computer (chip menu) → the stream connects, Ada's playhead follows Hal's clock and
// her transport shows Hal playing → Ada's Stop/Play/Locate go to Hal as TransportRequests →
// Hal leaves → Ada is back on her stopped local transport with the reason shown.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project, TransportState } from "@/generated";
import { launch, openRelayJoin, playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `e2e-listen-${Math.random().toString(36).slice(2, 8)}`;
const HOST_SITE = "777000111";
/** `ETHER_SCREENSHOTS=<dir>`: also save PR screenshots (1440×900, dark theme) there. */
const SHOTS = process.env.ETHER_SCREENSHOTS ?? "";

async function shot(page: Page, name: string, alsoNarrow = false): Promise<void> {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: resolve(SHOTS, `${name}.png`), animations: "disabled" });
  if (!alsoNarrow) return;
  // The top bar is tight: check the badge at a smaller laptop width too.
  const size = page.viewportSize()!;
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.screenshot({ path: resolve(SHOTS, `${name}-1280.png`), animations: "disabled" });
  await page.setViewportSize(size);
}

// Same-browser WebRTC on loopback: expose host candidates instead of mDNS names.
test.use({
  launchOptions: {
    args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--disable-features=WebRtcHideLocalIpsWithMdns"],
  },
});

interface Handle {
  state(): { project: Project | null; transport: TransportState | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
const transportState = (page: Page): Promise<TransportState | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().transport);

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
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function join(page: Page, name: string) {
  await openRelayJoin(page);
  await page.getByLabel("Relay address").fill(relayUrl);
  await page.getByLabel("Session").fill(SESSION);
  await page.getByLabel("Your name").fill(name);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });
}

/** What the scripted host exposes on `window.__host`. */
interface ScriptedHost {
  log: { type: string; [k: string]: unknown }[];
  anchors: number;
  setCanHost(can: boolean): void;
  leave(): void;
}

/**
 * The scripted host (runs in its own page): relay handshake, presence with `can_host`, and
 * on `Listen` a send-only peer connection with a 440 Hz tone, trickle ICE, and stream clock
 * anchors (120 bpm, playing from beat 16) every 100 ms from the RTP timestamps it sends.
 */
async function startScriptedHost(page: Page, args: { url: string; token: string; site: string }): Promise<void> {
  await page.evaluate(async ({ url, token, site }) => {
    type Msg = { type: string; [k: string]: unknown };
    const host = { log: [] as Msg[], anchors: 0, leave: () => undefined as void, setCanHost: (_: boolean) => undefined as void };
    (window as unknown as { __host: typeof host }).__host = host;
    const ws = new WebSocket(url);
    const send = (m: Msg) => ws.send(JSON.stringify(m));
    await new Promise<void>((ok, fail) => {
      ws.onopen = () => {
        ws.send(JSON.stringify({ protocol_version: 1, token, client: "scripted host (e2e)" }));
      };
      ws.onerror = () => fail(new Error("relay connection failed"));
      ws.onmessage = (e) => {
        const hello = JSON.parse(e.data as string) as { type: string };
        if (hello.type !== "Welcome") fail(new Error(`relay refused: ${e.data}`));
        ok();
      };
    });
    const presenceState = {
      cursor: null,
      selected_tracks: [],
      selected_clips: [],
      selected_notes: [],
      selected_devices: [],
      view: null,
      can_host: true,
    };
    // `ether_collab::wire::COLLAB_PROTOCOL_VERSION` (2 since base-62 social); the relay refuses others.
    send({ type: "Hello", site, actor: null, name: "Hal", protocol_version: 2 });
    send({ type: "SyncRequest", site, version: "" });
    host.setCanHost = (can: boolean) =>
      send({ type: "Presence", presence: { site, actor: null, name: "Hal", color: 0, state: { ...presenceState, can_host: can } } });
    host.setCanHost(false);
    host.leave = () => {
      send({ type: "Leave", site });
      ws.close();
    };

    let pc: RTCPeerConnection | null = null;
    let listener = "";
    let stream = 0;
    const t0 = performance.now();
    const positionAt = (t: number) => 16 + ((t - t0) / 1000) * 2; // 120 bpm from beat 16
    let lastRtp: { rtp: number; at: number } | null = null;

    ws.onmessage = async (e) => {
      if (typeof e.data !== "string") return;
      const m = JSON.parse(e.data) as Msg;
      if (m.to !== site) return;
      host.log.push(m);
      if (m.type === "Listen") {
        listener = m.from as string;
        stream = m.stream as number;
        pc = new RTCPeerConnection({ encodedInsertableStreams: true } as RTCConfiguration);
        const ctx = new AudioContext();
        const osc = ctx.createOscillator();
        const dest = ctx.createMediaStreamDestination();
        osc.connect(dest);
        osc.start();
        const sender = pc.addTrack(dest.stream.getAudioTracks()[0]!, dest.stream);
        // Observe the RTP timestamp of every encoded frame (read-only).
        const streams = (
          sender as unknown as { createEncodedStreams(): { readable: ReadableStream; writable: WritableStream } }
        ).createEncodedStreams();
        void streams.readable
          .pipeThrough(
            new TransformStream({
              transform(frame: { timestamp: number; getMetadata?: () => { rtpTimestamp?: number } }, ctl) {
                const rtp = frame.getMetadata?.().rtpTimestamp ?? frame.timestamp;
                lastRtp = { rtp: rtp >>> 0, at: performance.now() };
                ctl.enqueue(frame);
              },
            }),
          )
          .pipeTo(streams.writable);
        pc.onicecandidate = (ev) => {
          const c = ev.candidate;
          send({
            type: "Signal",
            from: site,
            to: listener,
            stream,
            signal: {
              type: "Ice",
              candidate: {
                candidate: c?.candidate ?? "",
                sdp_mid: c?.sdpMid ?? null,
                sdp_m_line_index: c?.sdpMLineIndex ?? null,
                username_fragment: c?.usernameFragment ?? null,
              },
            },
          });
        };
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        send({ type: "Signal", from: site, to: listener, stream, signal: { type: "Offer", sdp: offer.sdp } });
        setInterval(() => {
          if (!lastRtp) return;
          host.anchors++;
          send({
            type: "StreamClock",
            from: site,
            to: listener,
            stream,
            clock: {
              rtp: lastRtp.rtp,
              position: positionAt(lastRtp.at),
              playing: true,
              recording: false,
              bpm: 120,
              loop_enabled: false,
              loop_region: { start: 0, end: 16 },
              metronome: false,
              discontinuity: host.anchors === 1,
            },
          });
        }, 100);
      } else if (m.type === "Signal" && pc && m.stream === stream) {
        const signal = m.signal as {
          type: string;
          sdp?: string;
          candidate?: { candidate: string; sdp_mid: string | null; sdp_m_line_index: number | null };
        };
        if (signal.type === "Answer") await pc.setRemoteDescription({ type: "answer", sdp: signal.sdp! });
        else if (signal.type === "Ice" && signal.candidate)
          await pc.addIceCandidate({
            candidate: signal.candidate.candidate,
            sdpMid: signal.candidate.sdp_mid,
            sdpMLineIndex: signal.candidate.sdp_m_line_index,
          });
      }
    };
  }, args);
}

const hostLog = (page: Page) => page.evaluate(() => (window as unknown as { __host: ScriptedHost }).__host.log);

test("listen on a scripted host: stream, shared playhead, forwarded transport, host leaves", async ({ browser }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  const ctxA = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const ctxH = await browser.newContext();
  const a = await ctxA.newPage();
  const h = await ctxH.newPage();
  a.on("pageerror", (e) => errors.push(e.message));

  await open(a);
  await a.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await join(a, "Ada");
  await a.keyboard.press("Escape");
  await startScriptedHost(h, { url: `${relayUrl}/${SESSION}`, token: TOKEN, site: HOST_SITE });
  const chip = a.locator('[data-testid="collab-peers"] [data-peer="Hal"]');
  await expect(chip).toBeVisible({ timeout: 10_000 });

  // --- Hal cannot host yet: the entry is disabled with the reason.
  await chip.click({ button: "right" });
  const blocked = a.getByRole("menuitem", { name: "Listen on Hal's computer (Hal can't host)" });
  await expect(blocked).toBeDisabled();
  await shot(a, "listen-disabled-menu");
  await a.keyboard.press("Escape");
  await a.getByTestId("collab-button").click();
  await expect(a.getByRole("button", { name: "Listen on Hal's computer" })).toBeDisabled();
  await shot(a, "listen-disabled-dialog");
  await a.keyboard.press("Escape");
  const host = (page: Page) => page.evaluate(() => (window as unknown as { __host: ScriptedHost }).__host.setCanHost(true));
  await host(h);

  // --- Listen from the chip menu.
  await chip.click({ button: "right" });
  await expect(a.getByRole("menuitem", { name: "Listen on Hal's computer" })).toBeEnabled({ timeout: 10_000 });
  await shot(a, "listen-menu");
  await a.getByRole("menuitem", { name: "Listen on Hal's computer" }).click();
  await expect.poll(async () => (await hostLog(h)).some((m) => m.type === "Listen"), { timeout: 10_000 }).toBe(true);
  const status = a.getByTestId("listen-status");
  await expect(status).toHaveAttribute("aria-label", "Listening to Hal", { timeout: 30_000 });

  // --- The transport shows the host playing; the playhead follows its clock (beat 16+).
  await expect.poll(async () => (await transportState(a))?.playing, { timeout: 10_000 }).toBe(true);
  const transportStop = a.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true }).first();
  await expect(transportStop).toBeVisible();
  await expect
    .poll(async () => Number((await a.getByTestId("position-bars").textContent())?.split(".")[0] ?? 0), { timeout: 10_000 })
    .toBeGreaterThanOrEqual(5);
  await shot(a, "listen-listening", true);
  const bar1 = await a.getByTestId("position-bars").textContent();
  await expect.poll(async () => a.getByTestId("position-bars").textContent(), { timeout: 10_000 }).not.toBe(bar1);

  // --- Transport commands go to the host.
  await transportStop.click();
  await expect
    .poll(async () => (await hostLog(h)).filter((m) => m.type === "TransportRequest").map((m) => (m.request as { type: string }).type), {
      timeout: 10_000,
    })
    .toContain("Stop");
  // (The scripted host ignores it and keeps playing: last command handled by the host wins.)
  await a.keyboard.press("Space");
  await expect
    .poll(async () => (await hostLog(h)).filter((m) => m.type === "TransportRequest").length, { timeout: 10_000 })
    .toBeGreaterThanOrEqual(2);

  // --- The host leaves: Ada is back on her stopped local transport, the reason is shown.
  await h.evaluate(() => (window as unknown as { __host: ScriptedHost }).__host.leave());
  await expect(status).toHaveAttribute("aria-label", "Stopped listening to Hal: Hal left", { timeout: 10_000 });
  await expect(status).toContainText("Hal left");
  await expect.poll(async () => (await transportState(a))?.playing, { timeout: 10_000 }).toBe(false);
  await expect(playButton(a)).toBeVisible();
  await shot(a, "listen-ended", true);
  await status.getByRole("button", { name: "Dismiss" }).click();
  await expect(status).toHaveCount(0);

  await ctxA.close();
  await ctxH.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
