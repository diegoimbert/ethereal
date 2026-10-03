/**
 * The Cloudflare side of the service: the `SignalRoom` Durable Object (one per room id,
 * driving `RoomCore` over the WebSocket hibernation API, so an idle room costs nothing) and
 * the `IpBudget` Durable Object (one per client IP: claims and bad doors, docs/SHARING.md §3.4).
 */

import { IpBudgets, type BudgetKind, type BudgetOp } from "./budget.ts";
import { IceProvider } from "./ice.ts";
import { originAllowed, route, sha256Hex } from "./protocol.ts";
import { newSocketState, RoomCore, type RoomRecord, type RoomSocket, type SocketState } from "./room.ts";

const DAY_MS = 86_400_000;

/** `ROOM_TTL_DAYS` in ms (30 days when unset or invalid). */
export function ttlMs(env: Pick<Env, "ROOM_TTL_DAYS">): number {
  const days = Number(env.ROOM_TTL_DAYS);
  return (Number.isFinite(days) && days > 0 ? days : 30) * DAY_MS;
}

/** The checks every front door shares (the Worker, the Pages Function → this object). */
export function upgradeError(request: Request, env: Pick<Env, "ALLOWED_ORIGINS">): Response | null {
  if (!originAllowed(request.headers.get("Origin"), env.ALLOWED_ORIGINS)) return new Response("origin not allowed", { status: 403 });
  if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
    return new Response("expected a WebSocket upgrade", { status: 426 });
  }
  return null;
}

export class SignalRoom {
  private readonly ctx: DurableObjectState;
  private readonly env: Env;
  private readonly ice: IceProvider;
  private core: RoomCore | null = null;
  /** Stable `RoomSocket` per WebSocket (the core compares sockets by identity). */
  private readonly sockets = new Map<CfWebSocket, RoomSocket>();

  constructor(ctx: DurableObjectState, env: Env) {
    this.ctx = ctx;
    this.env = env;
    this.ice = new IceProvider(env, (input, init) => fetch(input, init), () => Date.now());
  }

  private wrap(ws: CfWebSocket, state: SocketState): RoomSocket {
    let sock = this.sockets.get(ws);
    if (!sock) {
      sock = {
        state,
        send: (m) => {
          try {
            ws.send(JSON.stringify(m));
          } catch {
            // Already closing: the close event cleans up.
          }
        },
        close: (code, reason) => {
          try {
            ws.close(code, reason);
          } catch {
            // Already closed.
          }
          this.sockets.delete(ws);
        },
        save: () => ws.serializeAttachment(state),
      };
      this.sockets.set(ws, sock);
    }
    return sock;
  }

  /** The socket for an event; after hibernation, rebuilt from its attachment. */
  private socket(ws: CfWebSocket): RoomSocket {
    return this.sockets.get(ws) ?? this.wrap(ws, ws.deserializeAttachment() as SocketState);
  }

  private async getCore(): Promise<RoomCore> {
    if (!this.core) {
      const record = (await this.ctx.storage.get<RoomRecord>("room")) ?? null;
      const sockets = this.ctx.getWebSockets().map((ws) => this.socket(ws));
      this.core = new RoomCore(
        record,
        {
          persist: async (r) => {
            if (r) await this.ctx.storage.put("room", r);
            else await this.ctx.storage.delete("room");
          },
          schedule: (at) => {
            // One alarm per object: the earliest of every deadline (hello, offline wait, TTL).
            void (at === null ? this.ctx.storage.deleteAlarm() : this.ctx.storage.setAlarm(at));
          },
          iceServers: () => this.ice.servers(),
          budget: (kind, ip, op) => spendBudget(this.env, kind, ip, op),
          sha256: sha256Hex,
          now: () => Date.now(),
        },
        { ttlMs: ttlMs(this.env), sockets },
      );
    }
    return this.core;
  }

  /** WebSocket upgrade, forwarded by the Worker or the Pages Function. */
  async fetch(request: Request): Promise<Response> {
    const r = route(new URL(request.url).pathname);
    if (!r || r.kind === "health") return new Response("not found", { status: 404 });
    const err = upgradeError(request, this.env);
    if (err) return err;
    const core = await this.getCore();
    const pair = new WebSocketPair();
    const ws = pair[1];
    this.ctx.acceptWebSocket(ws, [r.kind]);
    const state = newSocketState(r.kind, request.headers.get("CF-Connecting-IP"), Date.now());
    const sock = this.wrap(ws, state);
    sock.save();
    await core.open(sock);
    return new Response(null, { status: 101, webSocket: pair[0] } as CfResponseInit);
  }

  async webSocketMessage(ws: CfWebSocket, message: string | ArrayBuffer): Promise<void> {
    const core = await this.getCore();
    const sock = this.socket(ws);
    // Binary frames are not part of the protocol: same as a malformed text frame.
    await core.message(sock, typeof message === "string" ? message : "\u0000");
  }

  async webSocketClose(ws: CfWebSocket): Promise<void> {
    const core = await this.getCore();
    await core.closed(this.socket(ws));
    this.sockets.delete(ws);
    try {
      ws.close(1000, "closed");
    } catch {
      // Already closed (or the runtime answered the close itself).
    }
  }

  async webSocketError(ws: CfWebSocket): Promise<void> {
    await this.webSocketClose(ws);
  }

  /** Deadlines and the TTL (`RoomCore.nextDeadline`). */
  async alarm(): Promise<void> {
    await (await this.getCore()).tick();
  }
}

/** Ask the `IpBudget` object for `ip`. Fails open: an outage never blocks sharing. */
async function spendBudget(env: Env, kind: BudgetKind, ip: string, op: BudgetOp): Promise<boolean> {
  if (!env.BUDGETS) return true;
  try {
    const stub = env.BUDGETS.get(env.BUDGETS.idFromName(ip));
    const res = await stub.fetch(new Request(`https://budget/${kind}/${op}`, { method: "POST" }));
    return ((await res.json()) as { ok: boolean }).ok !== false;
  } catch {
    return true;
  }
}

/** One per client IP: sliding windows of claims and bad doors, kept in storage. */
export class IpBudget {
  private readonly ctx: DurableObjectState;
  private budgets: IpBudgets | null = null;

  constructor(ctx: DurableObjectState, _env: Env) {
    this.ctx = ctx;
  }

  async fetch(request: Request): Promise<Response> {
    const [, kind, op] = new URL(request.url).pathname.split("/");
    if ((kind !== "claim" && kind !== "badDoor") || (op !== "peek" && op !== "spend")) {
      return new Response("not found", { status: 404 });
    }
    const now = Date.now();
    this.budgets ??= new IpBudgets(await this.ctx.storage.get<Record<string, number[]>>("hits"));
    const ok = this.budgets.check(kind, "ip", op, now);
    if (op === "spend") {
      this.budgets.sweep(now);
      await this.ctx.storage.put("hits", this.budgets.toJSON());
      // Nothing left to remember after an hour: let the storage go.
      await this.ctx.storage.setAlarm(now + 3_600_000);
    }
    return Response.json({ ok });
  }

  async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
    this.budgets = null;
  }
}
