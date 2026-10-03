/**
 * End-to-end smoke test of a running signaling service: `wrangler dev`, the Node adapter, or
 * a deployment. It plays a host and joiners over real WebSockets.
 *
 *   node services/signal/test/smoke.ts http://127.0.0.1:8787
 *   node services/signal/test/smoke.ts https://etherealws.pages.dev/signal
 *
 * Exits 0 and prints `smoke: ok` when every step passed.
 */

import { pathToFileURL } from "node:url";
import { sha256Hex, type SignalClientMessage, type SignalServerMessage } from "../src/protocol.ts";

const b64url = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const random = (n: number) => b64url(crypto.getRandomValues(new Uint8Array(n)));

/** One WebSocket with a queue of parsed frames. */
export class SmokeClient {
  private readonly ws: WebSocket;
  private readonly queue: SignalServerMessage[] = [];
  private waiter: (() => void) | null = null;
  readonly opened: Promise<void>;
  readonly closed: Promise<{ code: number; reason: string }>;

  constructor(url: string) {
    this.ws = new WebSocket(url);
    this.opened = new Promise((resolve, reject) => {
      this.ws.addEventListener("open", () => resolve());
      this.ws.addEventListener("error", () => reject(new Error(`cannot connect to ${url}`)));
    });
    this.closed = new Promise((resolve) => {
      this.ws.addEventListener("close", (e) => {
        resolve({ code: e.code, reason: e.reason });
        this.waiter?.();
      });
    });
    this.ws.addEventListener("message", (e) => {
      this.queue.push(JSON.parse(String(e.data)) as SignalServerMessage);
      this.waiter?.();
    });
  }

  send(m: SignalClientMessage): void {
    this.ws.send(JSON.stringify(m));
  }

  /** The next frame; it must be of `type`. */
  async next<T extends SignalServerMessage["type"]>(type: T, timeoutMs = 5000): Promise<Extract<SignalServerMessage, { type: T }>> {
    const deadline = Date.now() + timeoutMs;
    while (!this.queue.length) {
      if (this.ws.readyState === WebSocket.CLOSED) throw new Error(`socket closed while waiting for ${type}`);
      const left = deadline - Date.now();
      if (left <= 0) throw new Error(`timed out waiting for ${type}`);
      await new Promise<void>((resolve) => {
        const t = setTimeout(resolve, left);
        this.waiter = () => {
          clearTimeout(t);
          resolve();
        };
      });
      this.waiter = null;
    }
    const m = this.queue.shift() as SignalServerMessage;
    if (m.type !== type) throw new Error(`expected ${type}, got ${JSON.stringify(m)}`);
    return m as Extract<SignalServerMessage, { type: T }>;
  }

  close(): void {
    this.ws.close(1000);
  }
}

function check(cond: unknown, what: string): asserts cond {
  if (!cond) throw new Error(`smoke: ${what}`);
}

/** Run every step against `base` (`http(s)://host[:port][/signal]`). */
export async function smoke(base: string, log: (line: string) => void = () => undefined): Promise<void> {
  const http = base.replace(/\/+$/, "");
  const ws = http.replace(/^http/, "ws");

  const health = (await (await fetch(`${http}/v1/health`)).json()) as { ok: boolean; protocol: number };
  check(health.ok && health.protocol === 1, `health answered ${JSON.stringify(health)}`);
  log("health ok");

  const room = random(16);
  const token = random(32);
  const door = random(16);
  const doorHash = await sha256Hex(door);
  const open = async (kind: "host" | "join") => {
    const c = new SmokeClient(`${ws}/v1/rooms/${room}/${kind}`);
    await c.opened;
    return c;
  };
  const host = async () => {
    const h = await open("host");
    h.send({ type: "HostHello", protocol: 1, host_token: token, doors: [doorHash], app: "smoke" });
    await h.next("HostWelcome");
    return h;
  };
  const join = async (d = door) => {
    const j = await open("join");
    j.send({ type: "JoinHello", protocol: 1, door: d, app: "smoke" });
    return j;
  };

  let h = await host();
  log("host claimed the room");

  const bad = await join(random(16));
  check((await bad.next("Refused")).reason === "InvalidInvite", "a wrong door is refused");
  await bad.closed;
  log("wrong door refused");

  const a = await join();
  const welcome = await a.next("JoinWelcome");
  const arrived = await h.next("PeerArrived");
  check(arrived.peer === welcome.peer, "PeerArrived names the joiner's peer");
  a.send({ type: "Signal", peer: 12345, signal: { type: "Offer", sdp: "v=0 smoke-offer" } });
  const offer = await h.next("Signal");
  check(offer.peer === welcome.peer && offer.signal.type === "Offer", "the joiner's offer reaches the host, stamped");
  h.send({ type: "Signal", peer: welcome.peer, signal: { type: "Answer", sdp: "v=0 smoke-answer" } });
  check((await a.next("Signal")).signal.type === "Answer", "the host's answer reaches the joiner");
  h.send({ type: "EndPeer", peer: welcome.peer, reason: "connected" });
  check((await a.closed).code === 4003, "EndPeer closes the joiner (4003)");
  log("introduction, signals and EndPeer ok");

  h.close();
  await h.closed;
  const waiting = await join();
  await waiting.next("HostOffline");
  h = await host();
  const late = await waiting.next("JoinWelcome");
  check((await h.next("PeerArrived")).peer === late.peer, "a waiting joiner is introduced when the host returns");
  waiting.close();
  check((await h.next("PeerLeft")).peer === late.peer, "PeerLeft when a paired joiner leaves");
  log("offline wait and late host ok");

  const last = await join();
  await last.next("JoinWelcome");
  await h.next("PeerArrived");
  h.send({ type: "CloseRoom" });
  check((await last.next("Refused")).reason === "InvalidInvite", "CloseRoom refuses joiners");
  await h.closed;
  const after = await join();
  check((await after.next("Refused")).reason === "InvalidInvite", "a closed room is forgotten");
  log("CloseRoom ok");
}

// CLI: `node services/signal/test/smoke.ts <base url>`.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const base = process.argv[2] ?? "http://127.0.0.1:8787";
  try {
    await smoke(base, (line) => console.log(`  ${line}`));
    console.log("smoke: ok");
  } catch (e) {
    console.error(e instanceof Error ? e.message : e);
    process.exit(1);
  }
}
