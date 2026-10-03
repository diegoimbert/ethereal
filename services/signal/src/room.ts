/**
 * One room (docs/SHARING.md §3.3) as a pure state machine: `RoomCore`. It has no
 * Workers-specific code, so the Durable Object (`do.ts`), the Node adapter
 * (`test/server.ts`) and the unit tests all drive the same logic.
 *
 * Persisted per room (and nothing else): SHA-256 of the host token, SHA-256 of each door,
 * when the host was last connected. Signals are forwarded, never stored.
 *
 * Every socket's own state (`SocketState`) is plain JSON, so the Durable Object can keep it
 * in the socket's attachment and rebuild the core after hibernation.
 *
 * (No TypeScript parameter properties or enums here: Node runs this file with type
 * stripping for the e2e adapter.)
 */

import {
  LIMITS,
  parseClientMessage,
  refused,
  SIGNAL_PROTOCOL_VERSION,
  type SignalClientMessage,
  type SignalRefusal,
  type SignalServerMessage,
} from "./protocol.ts";
import type { BudgetKind, BudgetOp } from "./budget.ts";
import type { IceServer } from "./ice.ts";

/** What a room persists. */
export interface RoomRecord {
  hostTokenHash: string;
  /** SHA-256 hex of each valid door. */
  doors: string[];
  lastHostSeenMs: number;
}

/** Close codes the service uses (after a `Refused`, the reason is the `SignalRefusal`). */
export const CLOSE = {
  /** `CloseRoom`: the host stopped sharing. */
  normal: 1000,
  /** After `Refused { reason }`. */
  refused: 4000,
  /** The host socket was replaced by a newer one with the same token (the app restarted). */
  replaced: 4001,
  /** No hello within `helloTimeoutMs`, or the host stayed offline for `waitForHostMs`. */
  timeout: 4002,
  /** `EndPeer` from the host. */
  ended: 4003,
} as const;

/** One socket's state; JSON so it survives hibernation (Durable Object attachments). */
export interface SocketState {
  kind: "host" | "join";
  /** Client IP for the per-IP budgets (`null`: unknown, e.g. tests). */
  ip: string | null;
  openedAt: number;
  /** The hello was accepted. */
  hello: boolean;
  /** Joiner: its pairing id (assigned on a valid `JoinHello`). */
  peer: number | null;
  /** Joiner: SHA-256 of the door it used (dropped when `SetDoors` removes it). */
  doorHash: string | null;
  /** Joiner: waiting for the host since (`HostOffline` sent), else `null` (paired). */
  waitingSince: number | null;
  /** Joiner: signals in the current pairing, both directions. */
  signals: number;
  /** Token bucket (frames/s). */
  tokens: number;
  tokensAt: number;
  /** The service closed it (a late close event is ignored). */
  closed: boolean;
}

export function newSocketState(kind: "host" | "join", ip: string | null, now: number): SocketState {
  return {
    kind,
    ip,
    openedAt: now,
    hello: false,
    peer: null,
    doorHash: null,
    waitingSince: null,
    signals: 0,
    tokens: LIMITS.framesPerSecond,
    tokensAt: now,
    closed: false,
  };
}

/** A connected socket as the core sees it. */
export interface RoomSocket {
  readonly state: SocketState;
  send(message: SignalServerMessage): void;
  close(code: number, reason: string): void;
  /** `state` changed (the Durable Object stores it in the socket's attachment). */
  save(): void;
}

/** Side effects the core asks for (the Durable Object or the Node adapter performs them). */
export interface RoomEffects {
  persist(record: RoomRecord | null): Promise<void>;
  /** The next time `tick()` must run (`null`: nothing pending). */
  schedule(atMs: number | null): void;
  iceServers(): Promise<IceServer[]>;
  /** Per-IP budgets (`budget.ts`); `true` = allowed. */
  budget(kind: BudgetKind, ip: string, op: BudgetOp): Promise<boolean>;
  sha256(text: string): Promise<string>;
  now(): number;
}

export interface RoomOptions {
  /** The room is deleted this long after the host was last connected. */
  ttlMs: number;
  /** Sockets still open after hibernation (their `state` says what they are). */
  sockets?: RoomSocket[];
}

/** Constant-time compare of two equal-length hex strings. */
function sameHash(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

/** WebSocket close reasons are at most 123 bytes. */
const closeReason = (s: string): string => (s.length > 100 ? `${s.slice(0, 100)}…` : s);

/** At most this many host sockets may be open before their hello. */
const MAX_PENDING_HOSTS = 4;

export class RoomCore {
  private record: RoomRecord | null;
  private readonly effects: RoomEffects;
  private readonly ttlMs: number;
  private host: RoomSocket | null = null;
  /** Joiners past their hello, waiting or paired, by peer id. */
  private readonly joiners = new Map<number, RoomSocket>();
  /** Sockets before their hello. */
  private readonly pending = new Set<RoomSocket>();
  private nextPeer = 1;
  /** Every event runs to completion before the next (hashing and storage are async). */
  private queue: Promise<void> = Promise.resolve();
  /** Storage writes started by synchronous bookkeeping, awaited before the event ends. */
  private writes: Promise<void>[] = [];

  constructor(record: RoomRecord | null, effects: RoomEffects, options: RoomOptions) {
    this.record = record;
    this.effects = effects;
    this.ttlMs = options.ttlMs;
    for (const s of options.sockets ?? []) {
      if (s.state.closed) continue;
      if (!s.state.hello) this.pending.add(s);
      else if (s.state.kind === "host") this.host = s;
      else if (s.state.peer !== null) {
        this.joiners.set(s.state.peer, s);
        this.nextPeer = Math.max(this.nextPeer, s.state.peer + 1);
      }
    }
  }

  /** The current persisted record (tests). */
  get state(): RoomRecord | null {
    return this.record;
  }

  /** Open sockets (tests and the Node adapter's cleanup). */
  get socketCount(): number {
    return this.pending.size + this.joiners.size + (this.host ? 1 : 0);
  }

  /** A socket was accepted. */
  open(socket: RoomSocket): Promise<void> {
    return this.run(() => {
      const now = this.effects.now();
      const pendingOfKind = [...this.pending].filter((s) => s.state.kind === socket.state.kind).length;
      if (socket.state.kind === "join" && this.joiners.size + pendingOfKind >= LIMITS.maxJoiners) {
        return this.refuse(socket, "RoomFull", "too many people are joining this project right now", now);
      }
      if (socket.state.kind === "host" && pendingOfKind >= MAX_PENDING_HOSTS) {
        return this.refuse(socket, "RateLimited", "too many host connections", now);
      }
      this.pending.add(socket);
    });
  }

  /** One inbound text frame. */
  message(socket: RoomSocket, text: string): Promise<void> {
    return this.run(async () => {
      const s = socket.state;
      if (s.closed) return;
      const now = this.effects.now();
      // Token bucket: `framesPerSecond`, burst = rate.
      const rate = LIMITS.framesPerSecond;
      s.tokens = Math.min(rate, s.tokens + ((now - s.tokensAt) * rate) / 1000);
      s.tokensAt = now;
      if (s.tokens < 1) return this.refuse(socket, "RateLimited", "too many messages", now);
      s.tokens -= 1;
      socket.save();

      const msg = parseClientMessage(text);
      if (!msg) return this.refuse(socket, "Malformed", "malformed message", now);
      if ("protocol" in msg && msg.protocol !== SIGNAL_PROTOCOL_VERSION) {
        return this.refuse(socket, "Version", `signaling protocol ${SIGNAL_PROTOCOL_VERSION} expected`, now);
      }
      await this.handle(socket, msg, now);
    });
  }

  /** The client closed the socket (or it errored). */
  closed(socket: RoomSocket): Promise<void> {
    return this.run(() => {
      if (socket.state.closed) return;
      this.depart(socket, this.effects.now());
      socket.state.closed = true;
    });
  }

  /** Deadlines: hello timeouts, offline waits, the room TTL. Called at `schedule`d times. */
  tick(): Promise<void> {
    return this.run(() => undefined);
  }

  /** The earliest pending deadline (`null`: none). */
  nextDeadline(): number | null {
    const times: number[] = [];
    for (const s of this.pending) times.push(s.state.openedAt + LIMITS.helloTimeoutMs);
    for (const j of this.joiners.values()) {
      if (j.state.waitingSince !== null) times.push(j.state.waitingSince + LIMITS.waitForHostMs);
    }
    if (this.record && !this.host) times.push(this.record.lastHostSeenMs + this.ttlMs);
    return times.length ? Math.min(...times) : null;
  }

  // ── internals ──────────────────────────────────────────────────────────────────────

  /** Serialize `f` after every earlier event, expire deadlines first, reschedule after. */
  private run(f: () => void | Promise<void>): Promise<void> {
    const next = this.queue.then(async () => {
      await this.expire(this.effects.now());
      try {
        await f();
      } finally {
        const writes = this.writes;
        this.writes = [];
        await Promise.all(writes);
        this.effects.schedule(this.nextDeadline());
      }
    });
    // A failing event must not wedge the room.
    this.queue = next.catch(() => undefined);
    return next;
  }

  private async expire(now: number): Promise<void> {
    for (const s of [...this.pending]) {
      if (now >= s.state.openedAt + LIMITS.helloTimeoutMs) this.drop(s, CLOSE.timeout, "no hello", now);
    }
    for (const j of [...this.joiners.values()]) {
      if (j.state.waitingSince !== null && now >= j.state.waitingSince + LIMITS.waitForHostMs) {
        this.drop(j, CLOSE.timeout, "the host is offline", now);
      }
    }
    if (this.record && !this.host && now >= this.record.lastHostSeenMs + this.ttlMs) {
      await this.forget("this shared project expired", now);
    }
  }

  private async handle(socket: RoomSocket, msg: SignalClientMessage, now: number): Promise<void> {
    const s = socket.state;
    if (msg.type === "Ping") return socket.send({ type: "Pong" });
    if (!s.hello) {
      if (s.kind === "host" && msg.type === "HostHello") return this.hostHello(socket, msg.host_token, msg.doors, now);
      if (s.kind === "join" && msg.type === "JoinHello") return this.joinHello(socket, msg.door, now);
      return this.refuse(socket, "Malformed", "the first message must be the hello", now);
    }
    if (s.kind === "host" && socket === this.host) {
      switch (msg.type) {
        case "SetDoors":
          return this.setDoors(msg.doors, now);
        case "CloseRoom":
          return this.closeRoom(now);
        case "Signal":
          return this.fromHost(msg.peer, msg.signal, now);
        case "EndPeer": {
          const j = this.joiners.get(msg.peer);
          if (j) this.drop(j, CLOSE.ended, closeReason(msg.reason ?? "ended by the host"), now, false);
          return;
        }
      }
    }
    if (s.kind === "join" && msg.type === "Signal") return this.fromJoiner(socket, msg.signal, now);
    return this.refuse(socket, "Malformed", `unexpected ${msg.type}`, now);
  }

  private async hostHello(socket: RoomSocket, token: string, doors: string[], now: number): Promise<void> {
    const hash = await this.effects.sha256(token);
    if (!this.record) {
      if (socket.state.ip !== null && !(await this.effects.budget("claim", socket.state.ip, "spend"))) {
        return this.refuse(socket, "RateLimited", "too many rooms created from this address", now);
      }
      this.record = { hostTokenHash: hash, doors: [...new Set(doors)], lastHostSeenMs: now };
    } else if (!sameHash(this.record.hostTokenHash, hash)) {
      return this.refuse(socket, "NotHost", "this room belongs to another host", now);
    } else {
      this.record = { ...this.record, doors: [...new Set(doors)], lastHostSeenMs: now };
    }
    await this.effects.persist(this.record);
    const ice = await this.effects.iceServers();

    const old = this.host;
    this.pending.delete(socket);
    socket.state.hello = true;
    socket.save();
    this.host = socket;
    if (old) {
      // The app restarted: the newer socket wins. Joiners are introduced again below.
      old.state.closed = true;
      old.save();
      old.close(CLOSE.replaced, "replaced by a newer host connection");
    }
    socket.send({
      type: "HostWelcome",
      ice_servers: ice,
      // `u64` is generated as `bigint`, but the wire is a JSON number.
      room_ttl_s: Math.round(this.ttlMs / 1000) as unknown as bigint,
    });
    this.dropRevoked(now);
    for (const j of this.joiners.values()) this.introduce(j, ice);
  }

  private async joinHello(socket: RoomSocket, door: string, now: number): Promise<void> {
    const ip = socket.state.ip;
    if (ip !== null && !(await this.effects.budget("badDoor", ip, "peek"))) {
      return this.refuse(socket, "RateLimited", "too many invalid invites from this address", now);
    }
    const hash = await this.effects.sha256(door);
    // An unknown room and a wrong door get the same answer (no room enumeration).
    if (!this.record || !this.record.doors.includes(hash)) {
      if (ip !== null) await this.effects.budget("badDoor", ip, "spend");
      return this.refuse(socket, "InvalidInvite", "this invite link is not valid (anymore)", now);
    }
    if (this.joiners.size >= LIMITS.maxJoiners) {
      return this.refuse(socket, "RoomFull", "too many people are joining this project right now", now);
    }
    const s = socket.state;
    this.pending.delete(socket);
    s.hello = true;
    s.peer = this.nextPeer++;
    s.doorHash = hash;
    this.joiners.set(s.peer, socket);
    if (this.host) {
      this.introduce(socket, await this.effects.iceServers());
    } else {
      s.waitingSince = now;
      socket.save();
      socket.send({ type: "HostOffline", last_seen_ms: this.record.lastHostSeenMs });
    }
  }

  /** Pair a joiner with the (online) host: `JoinWelcome` to it, `PeerArrived` to the host. */
  private introduce(joiner: RoomSocket, ice: IceServer[]): void {
    const s = joiner.state;
    if (!this.host || s.peer === null) return;
    s.waitingSince = null;
    s.signals = 0;
    joiner.save();
    joiner.send({ type: "JoinWelcome", peer: s.peer, ice_servers: ice });
    this.host.send({ type: "PeerArrived", peer: s.peer });
  }

  private async setDoors(doors: string[], now: number): Promise<void> {
    if (!this.record) return;
    this.record = { ...this.record, doors: [...new Set(doors)] };
    await this.effects.persist(this.record);
    this.dropRevoked(now);
  }

  /** Joiners whose door was removed (link reset, member removed) are refused at once. */
  private dropRevoked(now: number): void {
    const doors = new Set(this.record?.doors ?? []);
    for (const j of [...this.joiners.values()]) {
      if (j.state.doorHash !== null && !doors.has(j.state.doorHash)) {
        this.refuse(j, "InvalidInvite", "this invite link was reset", now);
      }
    }
  }

  private async closeRoom(now: number): Promise<void> {
    await this.forget("the host stopped sharing this project", now);
    for (const s of [...this.pending]) this.drop(s, CLOSE.normal, "room closed", now);
    if (this.host) this.drop(this.host, CLOSE.normal, "room closed", now);
  }

  /** Delete the record (CloseRoom, TTL) and refuse every joiner. */
  private async forget(message: string, now: number): Promise<void> {
    this.record = null;
    await this.effects.persist(null);
    for (const j of [...this.joiners.values()]) this.refuse(j, "InvalidInvite", message, now);
  }

  private fromHost(peer: number, signal: Extract<SignalClientMessage, { type: "Signal" }>["signal"], now: number): void {
    const j = this.joiners.get(peer);
    // A signal for a pairing that just ended is a normal race: dropped.
    if (!j || j.state.waitingSince !== null) return;
    if (this.countSignal(j, now)) j.send({ type: "Signal", peer, signal });
  }

  private fromJoiner(socket: RoomSocket, signal: Extract<SignalClientMessage, { type: "Signal" }>["signal"], now: number): void {
    const peer = socket.state.peer;
    // Waiting for the host: there is no pairing yet, the signal is dropped. The joiner's own
    // `peer` is stamped, whatever it wrote.
    if (!this.host || peer === null || socket.state.waitingSince !== null) return;
    if (this.countSignal(socket, now)) this.host.send({ type: "Signal", peer, signal });
  }

  /** `false` (and the pairing ends) past `maxSignalsPerPeer`. */
  private countSignal(joiner: RoomSocket, now: number): boolean {
    joiner.state.signals += 1;
    joiner.save();
    if (joiner.state.signals <= LIMITS.maxSignalsPerPeer) return true;
    this.refuse(joiner, "RateLimited", "too many signals for one connection", now);
    return false;
  }

  /** `Refused { reason }`, then close. */
  private refuse(socket: RoomSocket, reason: SignalRefusal, message: string, now: number): void {
    if (socket.state.closed) return;
    socket.send(refused(reason, message));
    this.drop(socket, CLOSE.refused, reason, now);
  }

  /** Close a socket from the service side; the others hear about it like a client close. */
  private drop(socket: RoomSocket, code: number, reason: string, now: number, notifyHost = true): void {
    if (socket.state.closed) return;
    this.depart(socket, now, notifyHost);
    socket.state.closed = true;
    socket.save();
    socket.close(code, reason);
  }

  /** Bookkeeping when a socket goes away (either side closed it). */
  private depart(socket: RoomSocket, now: number, notifyHost = true): void {
    const s = socket.state;
    this.pending.delete(socket);
    if (socket === this.host) {
      this.host = null;
      if (this.record) {
        this.record = { ...this.record, lastHostSeenMs: now };
        this.writes.push(this.effects.persist(this.record));
      }
      // Paired and waiting joiners all wait for the host again (bounded by `waitForHostMs`).
      for (const j of this.joiners.values()) {
        j.state.waitingSince = now;
        j.state.signals = 0;
        j.save();
        j.send({ type: "HostOffline", last_seen_ms: now });
      }
    } else if (s.kind === "join" && s.peer !== null && this.joiners.get(s.peer) === socket) {
      this.joiners.delete(s.peer);
      if (notifyHost && this.host && s.waitingSince === null) this.host.send({ type: "PeerLeft", peer: s.peer });
    }
  }
}
