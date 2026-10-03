/**
 * One room (docs/SHARING.md §3.3): a pure state machine (`RoomCore`, unit-testable without
 * the Workers runtime) and the Durable Object that drives it (`SignalRoom`, WebSocket
 * hibernation API: an idle room costs nothing).
 *
 * Persisted per room (and nothing else): SHA-256 of the host token, SHA-256 of each door,
 * when the host was last connected. Signals are forwarded, never stored. The room is
 * deleted `ROOM_TTL_DAYS` after the host was last connected (alarm), or on `CloseRoom`.
 *
 * SKELETON (base-115): the transitions below are specified in docs/SHARING.md §3.3 and
 * implemented by node `signal-service`; until then every hello is refused.
 */

import { LIMITS, parseClientMessage, refused, sha256Hex, SIGNAL_PROTOCOL_VERSION, type SignalClientMessage, type SignalServerMessage } from "./protocol";

/** What a room persists. */
export interface RoomRecord {
  hostTokenHash: string;
  /** SHA-256 hex of each valid door. */
  doors: string[];
  lastHostSeenMs: number;
}

/** A connected socket as the core sees it. */
export interface RoomSocket {
  readonly kind: "host" | "join";
  /** Pairing id (joiners), assigned on a valid `JoinHello`. */
  peer: number | null;
  send(message: SignalServerMessage): void;
  close(code: number, reason: string): void;
}

/** Side effects the core asks for (the Durable Object performs them). */
export interface RoomEffects {
  persist(record: RoomRecord | null): Promise<void>;
  sha256(text: string): Promise<string>;
  now(): number;
}

export class RoomCore {
  constructor(
    private record: RoomRecord | null,
    private readonly effects: RoomEffects,
  ) {}

  /** The current persisted record (tests). */
  get state(): RoomRecord | null {
    return this.record;
  }

  /** One inbound frame (already size-limited by the transport). */
  async message(socket: RoomSocket, text: string): Promise<void> {
    const msg = parseClientMessage(text);
    if (!msg) return this.refuse(socket, "Malformed", "malformed message");
    if ("protocol" in msg && msg.protocol !== SIGNAL_PROTOCOL_VERSION) {
      return this.refuse(socket, "Version", `signaling protocol ${SIGNAL_PROTOCOL_VERSION} expected`);
    }
    await this.handle(socket, msg);
  }

  /** A socket closed. */
  closed(_socket: RoomSocket): void {
    // signal-service: host gone → joiners get `HostOffline`, paired peers `PeerLeft`.
  }

  private async handle(socket: RoomSocket, msg: SignalClientMessage): Promise<void> {
    if (msg.type === "Ping") return socket.send({ type: "Pong" });
    // signal-service: HostHello (claim / prove), SetDoors, CloseRoom, JoinHello (door check,
    // HostOffline wait, JoinWelcome + PeerArrived), Signal routing, EndPeer, limits.
    void LIMITS;
    void this.effects;
    this.refuse(socket, "Malformed", "the signaling service is not implemented yet");
  }

  private refuse(socket: RoomSocket, reason: Parameters<typeof refused>[0], message: string): void {
    socket.send(refused(reason, message));
    socket.close(4000, reason);
  }
}

/** The Durable Object (one per room id, `env.ROOMS.idFromName(room)`). */
export class SignalRoom {
  private core: RoomCore | null = null;

  constructor(
    private readonly ctx: DurableObjectState,
    private readonly env: Env,
  ) {}

  private async getCore(): Promise<RoomCore> {
    if (!this.core) {
      const record = (await this.ctx.storage.get<RoomRecord>("room")) ?? null;
      const ttlMs = Number(this.env.ROOM_TTL_DAYS || "30") * 86_400_000;
      this.core = new RoomCore(record, {
        persist: async (r) => {
          if (r) {
            await this.ctx.storage.put("room", r);
            await this.ctx.storage.setAlarm(r.lastHostSeenMs + ttlMs);
          } else await this.ctx.storage.deleteAll();
        },
        sha256: sha256Hex,
        now: () => Date.now(),
      });
    }
    return this.core;
  }

  /** WebSocket upgrade, forwarded by the Worker with `kind` in the URL. */
  async fetch(request: Request): Promise<Response> {
    const kind = new URL(request.url).pathname.endsWith("/host") ? "host" : "join";
    const pair = new WebSocketPair();
    this.ctx.acceptWebSocket(pair[1], [kind]);
    pair[1].serializeAttachment({ kind, peer: null });
    return new Response(null, { status: 101, webSocket: pair[0] } as CfResponseInit);
  }

  private socket(ws: CfWebSocket): RoomSocket {
    const att = ws.deserializeAttachment() as { kind: "host" | "join"; peer: number | null };
    return {
      kind: att.kind,
      peer: att.peer,
      send: (m) => ws.send(JSON.stringify(m)),
      close: (code, reason) => ws.close(code, reason),
    };
  }

  async webSocketMessage(ws: CfWebSocket, message: string | ArrayBuffer): Promise<void> {
    const sock = this.socket(ws);
    if (typeof message !== "string") return sock.close(4000, "Malformed");
    await (await this.getCore()).message(sock, message);
  }

  async webSocketClose(ws: CfWebSocket): Promise<void> {
    (await this.getCore()).closed(this.socket(ws));
  }

  /** TTL: the host has not connected for `ROOM_TTL_DAYS`. */
  async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
    this.core = null;
  }
}
