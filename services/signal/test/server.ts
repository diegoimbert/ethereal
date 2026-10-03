/**
 * The signaling service on Node (`ws`), for the e2e tests of the other share nodes and for
 * local development without Wrangler. It runs the same `RoomCore` as the Durable Object;
 * rooms live in memory.
 *
 *   node services/signal/test/server.ts [--port 8789] [--host 127.0.0.1]
 *     [--stun stun:host:3478,...] [--origins <ALLOWED_ORIGINS>] [--ip-budgets] [--trust-proxy]
 *
 * prints `signal: http://127.0.0.1:8789` once listening (point the app's Settings > Advanced
 * > Signaling server, or an invite's `?s=`, at that URL). Same routes as the Worker, also
 * under `/signal/...`.
 *
 * In code (Playwright global setup, Rust/TS integration tests):
 *
 *   const server = await startSignalServer({ port: 0 });
 *   ... server.url ...
 *   await server.close();
 *
 * Defaults suit tests: any Origin, no ICE servers (loopback host candidates are enough), and
 * no per-IP budgets (a test run makes many rooms from 127.0.0.1). Every other limit applies.
 */

import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { pathToFileURL } from "node:url";
import { WebSocketServer, type WebSocket } from "ws";
import { IpBudgets } from "../src/budget.ts";
import { stunServers, type IceServer } from "../src/ice.ts";
import { LIMITS, originAllowed, route, sha256Hex, SIGNAL_PROTOCOL_VERSION } from "../src/protocol.ts";
import { newSocketState, RoomCore, type RoomSocket } from "../src/room.ts";

export interface SignalServerOptions {
  /** `0` (default) = any free port. */
  port?: number;
  host?: string;
  /** Advertised in `HostWelcome`/`JoinWelcome` (default: none). */
  iceServers?: IceServer[];
  /** `ALLOWED_ORIGINS` syntax; `null` (default) = any browser origin. */
  allowedOrigins?: string | null;
  /** Room TTL (default 30 days). */
  ttlMs?: number;
  /** Per-IP claim and bad-door budgets (default off, see above). */
  ipBudgets?: boolean;
  /** Behind a reverse proxy: the client IP is the first `X-Forwarded-For` entry. */
  trustProxy?: boolean;
}

export interface SignalServer {
  /** `http://<host>:<port>`: the signaling URL the app and the clients use. */
  readonly url: string;
  readonly port: number;
  /** Rooms in memory (claimed or with open sockets). */
  roomCount(): number;
  close(): Promise<void>;
}

interface Room {
  core: RoomCore;
  timer: ReturnType<typeof setTimeout> | null;
}

export async function startSignalServer(options: SignalServerOptions = {}): Promise<SignalServer> {
  const host = options.host ?? "127.0.0.1";
  const ice = options.iceServers ?? [];
  const allowed = (origin: string | null) =>
    options.allowedOrigins == null || originAllowed(origin, options.allowedOrigins);
  const ttlMs = options.ttlMs ?? 30 * 86_400_000;
  const budgets = new IpBudgets();
  const rooms = new Map<string, Room>();
  const sockets = new Set<WebSocket>();

  /** Drop a room that holds nothing (no record, no sockets). */
  const gc = (id: string, room: Room) => {
    if (room.core.state === null && room.core.socketCount === 0 && rooms.get(id) === room) {
      if (room.timer) clearTimeout(room.timer);
      rooms.delete(id);
    }
  };

  const getRoom = (id: string): Room => {
    let room = rooms.get(id);
    if (room) return room;
    const r: Room = { core: null as unknown as RoomCore, timer: null };
    r.core = new RoomCore(
      null,
      {
        persist: async () => undefined, // the record lives in the core's memory
        schedule: (at) => {
          if (r.timer) clearTimeout(r.timer);
          r.timer = null;
          if (at === null) return;
          // Node timers cap at ~24.8 days; a longer TTL is simply re-checked then.
          const delay = Math.min(Math.max(0, at - Date.now()), 2 ** 31 - 1);
          r.timer = setTimeout(() => void r.core.tick().then(() => gc(id, r)), delay);
          r.timer.unref();
        },
        iceServers: async () => ice,
        budget: async (kind, ip, op) => !options.ipBudgets || budgets.check(kind, ip, op, Date.now()),
        sha256: sha256Hex,
        now: () => Date.now(),
      },
      { ttlMs },
    );
    room = r;
    rooms.set(id, room);
    return room;
  };

  const cors = (req: IncomingMessage): Record<string, string> => {
    const origin = req.headers.origin ?? null;
    return origin && allowed(origin) ? { "Access-Control-Allow-Origin": origin, Vary: "Origin" } : {};
  };

  const http = createServer((req: IncomingMessage, res: ServerResponse) => {
    const r = route(new URL(req.url ?? "/", "http://x").pathname);
    if (!r) return void res.writeHead(404).end("not found");
    if (!allowed(req.headers.origin ?? null)) return void res.writeHead(403).end("origin not allowed");
    if (r.kind !== "health") return void res.writeHead(426).end("expected a WebSocket upgrade");
    res.writeHead(200, { "Content-Type": "application/json", ...cors(req) });
    res.end(JSON.stringify({ ok: true, protocol: SIGNAL_PROTOCOL_VERSION }));
  });

  // Text frames are limited by `parseClientMessage`; this only bounds what `ws` buffers.
  const wss = new WebSocketServer({ noServer: true, maxPayload: 4 * LIMITS.maxFrameBytes });

  http.on("upgrade", (req, socket, head) => {
    const r = route(new URL(req.url ?? "/", "http://x").pathname);
    const reject = (status: string) => {
      socket.end(`HTTP/1.1 ${status}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`);
    };
    if (!r || r.kind === "health") return reject("404 Not Found");
    if (!allowed(req.headers.origin ?? null)) return reject("403 Forbidden");
    wss.handleUpgrade(req, socket, head, (ws) => {
      sockets.add(ws);
      const room = getRoom(r.room);
      const forwarded = options.trustProxy ? String(req.headers["x-forwarded-for"] ?? "").split(",")[0]?.trim() : "";
      const state = newSocketState(r.kind, forwarded || req.socket.remoteAddress || null, Date.now());
      const sock: RoomSocket = {
        state,
        send: (m) => {
          if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(m));
        },
        close: (code, reason) => ws.close(code, reason),
        save: () => undefined,
      };
      ws.on("message", (data, isBinary) => {
        // Binary frames are not part of the protocol: same as a malformed text frame.
        void room.core.message(sock, isBinary ? "\u0000" : data.toString());
      });
      ws.on("close", () => {
        sockets.delete(ws);
        void room.core.closed(sock).then(() => gc(r.room, room));
      });
      ws.on("error", () => ws.terminate());
      void room.core.open(sock);
    });
  });

  await new Promise<void>((resolve, reject) => {
    http.once("error", reject);
    http.listen(options.port ?? 0, host, () => resolve());
  });
  const port = (http.address() as AddressInfo).port;

  return {
    url: `http://${host.includes(":") ? `[${host}]` : host}:${port}`,
    port,
    roomCount: () => rooms.size,
    close: async () => {
      for (const room of rooms.values()) if (room.timer) clearTimeout(room.timer);
      for (const ws of sockets) ws.terminate();
      await new Promise<void>((resolve) => wss.close(() => resolve()));
      await new Promise<void>((resolve) => http.close(() => resolve()));
    },
  };
}

// CLI: `node services/signal/test/server.ts [--port N] [--host H]`.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const arg = (name: string) => {
    const i = process.argv.indexOf(`--${name}`);
    return i > 0 ? process.argv[i + 1] : undefined;
  };
  const server = await startSignalServer({
    port: Number(arg("port") ?? process.env.PORT ?? 8789),
    host: arg("host") ?? "127.0.0.1",
    iceServers: stunServers(arg("stun")),
    allowedOrigins: arg("origins") ?? null,
    ipBudgets: process.argv.includes("--ip-budgets"),
    trustProxy: process.argv.includes("--trust-proxy"),
  });
  console.log(`signal: ${server.url}`);
  const stop = () => void server.close().then(() => process.exit(0));
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);
}
