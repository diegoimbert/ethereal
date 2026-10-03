import { beforeAll, describe, expect, it } from "vitest";
import { IpBudgets } from "./budget.ts";
import type { IceServer } from "./ice.ts";
import { LIMITS, sha256Hex, type SignalClientMessage, type SignalServerMessage } from "./protocol.ts";
import { CLOSE, newSocketState, RoomCore, type RoomRecord, type RoomSocket, type SocketState } from "./room.ts";

const DAY = 86_400_000;
const TTL = 30 * DAY;
const TOKEN = "T".repeat(43);
const OTHER_TOKEN = "U".repeat(43);
const DOOR = "D".repeat(22);
const DOOR2 = "E".repeat(22);
const BAD_DOOR = "X".repeat(22);
const ICE: IceServer[] = [{ urls: ["stun:stun.example:3478"], username: null, credential: null }];
const OFFER = { type: "Offer" as const, sdp: "v=0 offer" };
const ANSWER = { type: "Answer" as const, sdp: "v=0 answer" };

let DOOR_HASH = "";
let DOOR2_HASH = "";
beforeAll(async () => {
  DOOR_HASH = await sha256Hex(DOOR);
  DOOR2_HASH = await sha256Hex(DOOR2);
});

class FakeSocket implements RoomSocket {
  readonly state: SocketState;
  readonly sent: SignalServerMessage[] = [];
  closedWith: { code: number; reason: string } | null = null;
  saves = 0;

  constructor(state: SocketState) {
    this.state = state;
  }
  send(m: SignalServerMessage): void {
    if (this.closedWith) throw new Error("send after close");
    this.sent.push(m);
  }
  close(code: number, reason: string): void {
    this.closedWith = { code, reason };
  }
  save(): void {
    this.saves++;
  }
  get last(): SignalServerMessage | undefined {
    return this.sent.at(-1);
  }
  types(): string[] {
    return this.sent.map((m) => m.type);
  }
}

/** A room with a fake clock, fake storage and shared per-IP budgets. */
function room(opts: { record?: RoomRecord | null; budgets?: IpBudgets; clock?: { t: number }; sockets?: RoomSocket[] } = {}) {
  const clock = opts.clock ?? { t: 1_700_000_000_000 };
  const budgets = opts.budgets ?? new IpBudgets();
  const persisted: (RoomRecord | null)[] = [];
  const h = {
    clock,
    persisted,
    scheduled: null as number | null,
    core: null as unknown as RoomCore,
    async open(kind: "host" | "join", ip: string | null = "10.0.0.1"): Promise<FakeSocket> {
      const s = new FakeSocket(newSocketState(kind, ip, clock.t));
      await h.core.open(s);
      return s;
    },
    send(s: FakeSocket, m: SignalClientMessage | Record<string, unknown>): Promise<void> {
      return h.core.message(s, JSON.stringify(m));
    },
    async host(token = TOKEN, doors = [DOOR_HASH], ip?: string): Promise<FakeSocket> {
      const s = await h.open("host", ip);
      await h.send(s, { type: "HostHello", protocol: 1, host_token: token, doors, app: "Ethereal test" });
      return s;
    },
    async join(door = DOOR, ip?: string): Promise<FakeSocket> {
      const s = await h.open("join", ip);
      await h.send(s, { type: "JoinHello", protocol: 1, door, app: "Ethereal test" });
      return s;
    },
    /** Move the clock and run the deadlines, like the alarm would. */
    async advance(ms: number): Promise<void> {
      clock.t += ms;
      await h.core.tick();
    },
  };
  h.core = new RoomCore(
    opts.record ?? null,
    {
      persist: async (r) => {
        persisted.push(r && { ...r, doors: [...r.doors] });
      },
      schedule: (at) => {
        h.scheduled = at;
      },
      iceServers: async () => ICE,
      budget: async (kind, ip, op) => budgets.check(kind, ip, op, clock.t),
      sha256: sha256Hex,
      now: () => clock.t,
    },
    { ttlMs: TTL, sockets: opts.sockets },
  );
  return h;
}

describe("claim and host", () => {
  it("the first HostHello claims the room and is welcomed with the ICE servers", async () => {
    const r = room();
    const host = await r.host();
    expect(host.sent).toEqual([{ type: "HostWelcome", ice_servers: ICE, room_ttl_s: 30 * 86_400 }]);
    expect(r.core.state).toEqual({ hostTokenHash: await sha256Hex(TOKEN), doors: [DOOR_HASH], lastHostSeenMs: r.clock.t });
    expect(r.persisted.at(-1)).toEqual(r.core.state);
    expect(host.closedWith).toBeNull();
  });

  it("refuses another token with NotHost and keeps the claim", async () => {
    const r = room();
    await r.host();
    const before = r.core.state;
    const other = await r.host(OTHER_TOKEN);
    expect(other.last).toMatchObject({ type: "Refused", reason: "NotHost" });
    expect(other.closedWith).toEqual({ code: CLOSE.refused, reason: "NotHost" });
    expect(r.core.state).toEqual(before);
  });

  it("a second host socket with the right token replaces the first and re-introduces joiners", async () => {
    const r = room();
    const first = await r.host();
    const joiner = await r.join();
    expect(joiner.last).toEqual({ type: "JoinWelcome", peer: 1, ice_servers: ICE });
    const second = await r.host(TOKEN, [DOOR_HASH, DOOR2_HASH]);
    expect(first.closedWith?.code).toBe(CLOSE.replaced);
    expect(second.types()).toEqual(["HostWelcome", "PeerArrived"]);
    expect(second.last).toEqual({ type: "PeerArrived", peer: 1 });
    expect(joiner.types()).toEqual(["JoinWelcome", "JoinWelcome"]);
    expect(r.core.state?.doors).toEqual([DOOR_HASH, DOOR2_HASH]);
    // The replaced socket's late close event changes nothing.
    await r.core.closed(first);
    expect(joiner.types()).toEqual(["JoinWelcome", "JoinWelcome"]);
  });

  it("refuses a wrong protocol version", async () => {
    const r = room();
    const host = await r.open("host");
    await r.send(host, { type: "HostHello", protocol: 2, host_token: TOKEN, doors: [], app: "x" });
    expect(host.last).toMatchObject({ type: "Refused", reason: "Version" });
    expect(r.core.state).toBeNull();
  });
});

describe("doors and joining", () => {
  it("a valid door with the host online: JoinWelcome to the joiner, PeerArrived to the host", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    const b = await r.join();
    expect(a.sent).toEqual([{ type: "JoinWelcome", peer: 1, ice_servers: ICE }]);
    expect(b.sent).toEqual([{ type: "JoinWelcome", peer: 2, ice_servers: ICE }]);
    expect(host.sent.slice(1)).toEqual([
      { type: "PeerArrived", peer: 1 },
      { type: "PeerArrived", peer: 2 },
    ]);
  });

  it("an unknown room and a wrong door get the same InvalidInvite", async () => {
    const unknown = room();
    const u = await unknown.join();
    const r = room();
    await r.host();
    const w = await r.join(BAD_DOOR);
    expect(u.last).toEqual(w.last);
    expect(w.last).toMatchObject({ type: "Refused", reason: "InvalidInvite" });
    expect(w.closedWith?.code).toBe(CLOSE.refused);
    expect(unknown.core.state).toBeNull();
  });

  it("host offline: the joiner waits with HostOffline, and is welcomed when the host connects", async () => {
    const r = room();
    const host = await r.host();
    const seen = r.clock.t + 5_000;
    r.clock.t = seen;
    await r.core.closed(host);
    expect(r.core.state?.lastHostSeenMs).toBe(seen);
    r.clock.t += 60_000;
    const joiner = await r.join();
    expect(joiner.sent).toEqual([{ type: "HostOffline", last_seen_ms: seen }]);
    expect(joiner.closedWith).toBeNull();
    // Signals while waiting go nowhere.
    await r.send(joiner, { type: "Signal", peer: 0, signal: OFFER });
    const back = await r.host();
    expect(back.sent.map((m) => m.type)).toEqual(["HostWelcome", "PeerArrived"]);
    expect(joiner.last).toEqual({ type: "JoinWelcome", peer: 1, ice_servers: ICE });
  });

  it("SetDoors replaces the doors and refuses joiners whose door was removed", async () => {
    const r = room();
    const host = await r.host();
    const old = await r.join(DOOR);
    await r.send(host, { type: "SetDoors", doors: [DOOR2_HASH] });
    expect(r.core.state?.doors).toEqual([DOOR2_HASH]);
    expect(r.persisted.at(-1)?.doors).toEqual([DOOR2_HASH]);
    expect(old.last).toMatchObject({ type: "Refused", reason: "InvalidInvite" });
    expect(host.last).toEqual({ type: "PeerLeft", peer: 1 });
    expect((await r.join(DOOR)).last).toMatchObject({ type: "Refused", reason: "InvalidInvite" });
    expect((await r.join(DOOR2)).last).toMatchObject({ type: "JoinWelcome", peer: 2 });
  });
});

describe("signals", () => {
  it("routes both ways and stamps the joiner's own peer", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    const b = await r.join();
    await r.send(a, { type: "Signal", peer: 99, signal: OFFER });
    expect(host.last).toEqual({ type: "Signal", peer: 1, signal: OFFER });
    await r.send(host, { type: "Signal", peer: 2, signal: ANSWER });
    expect(b.last).toEqual({ type: "Signal", peer: 2, signal: ANSWER });
    expect(a.last?.type).toBe("JoinWelcome");
    // A signal to a pairing that ended is dropped, not an error.
    await r.send(host, { type: "Signal", peer: 7, signal: ANSWER });
    expect(host.closedWith).toBeNull();
  });

  it("EndPeer closes that joiner without a PeerLeft", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    await r.send(host, { type: "EndPeer", peer: 1, reason: "connected" });
    expect(a.closedWith).toEqual({ code: CLOSE.ended, reason: "connected" });
    expect(host.types()).toEqual(["HostWelcome", "PeerArrived"]);
    await r.core.closed(a);
    expect(host.types()).toEqual(["HostWelcome", "PeerArrived"]);
  });

  it("a joiner closing ends its pairing (PeerLeft); a waiting joiner closing is silent", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    await r.core.closed(a);
    expect(host.last).toEqual({ type: "PeerLeft", peer: 1 });
    await r.core.closed(host);
    const waiting = await r.join();
    await r.core.closed(waiting);
    const back = await r.host();
    expect(back.types()).toEqual(["HostWelcome"]);
  });

  it("the host leaving puts paired joiners back to waiting", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    await r.core.closed(host);
    expect(a.last).toEqual({ type: "HostOffline", last_seen_ms: r.clock.t });
    expect(a.closedWith).toBeNull();
  });
});

describe("CloseRoom and TTL", () => {
  it("CloseRoom forgets the room and closes every socket", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    const pending = await r.open("join");
    await r.send(host, { type: "CloseRoom" });
    expect(r.core.state).toBeNull();
    expect(r.persisted.at(-1)).toBeNull();
    expect(a.last).toMatchObject({ type: "Refused", reason: "InvalidInvite" });
    expect(host.closedWith?.code).toBe(CLOSE.normal);
    expect(pending.closedWith?.code).toBe(CLOSE.normal);
    expect(r.core.socketCount).toBe(0);
    expect(r.scheduled).toBeNull();
    expect((await r.join()).last).toMatchObject({ type: "Refused", reason: "InvalidInvite" });
    // The room id is free again: a new host claims it with a new token.
    expect((await r.host(OTHER_TOKEN)).last?.type).toBe("HostWelcome");
  });

  it("the room is deleted ROOM_TTL after the host was last connected", async () => {
    const r = room();
    const host = await r.host();
    expect(r.scheduled).toBeNull(); // the host is online: no TTL running
    await r.core.closed(host);
    const seen = r.clock.t;
    expect(r.scheduled).toBe(seen + TTL);
    const waiting = await r.join();
    await r.advance(TTL - 1);
    expect(r.core.state).not.toBeNull();
    await r.advance(1);
    expect(r.core.state).toBeNull();
    expect(r.persisted.at(-1)).toBeNull();
    expect(r.scheduled).toBeNull();
    expect(waiting.closedWith?.code).toBe(CLOSE.timeout); // its 10-minute wait ended first
  });

  it("a host reconnecting restarts the TTL", async () => {
    const r = room();
    await r.core.closed(await r.host());
    await r.advance(TTL - 1000);
    const host = await r.host();
    await r.core.closed(host);
    await r.advance(TTL - 1);
    expect(r.core.state).not.toBeNull();
  });
});

describe("limits", () => {
  it("closes a socket without a hello after helloTimeoutMs", async () => {
    const r = room();
    const s = await r.open("join");
    expect(r.scheduled).toBe(r.clock.t + LIMITS.helloTimeoutMs);
    await r.advance(LIMITS.helloTimeoutMs - 1);
    expect(s.closedWith).toBeNull();
    await r.advance(1);
    expect(s.closedWith?.code).toBe(CLOSE.timeout);
  });

  it("closes a joiner that waited waitForHostMs for an offline host", async () => {
    const r = room();
    await r.core.closed(await r.host());
    const waiting = await r.join();
    expect(r.scheduled).toBe(r.clock.t + LIMITS.waitForHostMs);
    await r.advance(LIMITS.waitForHostMs);
    expect(waiting.closedWith?.code).toBe(CLOSE.timeout);
  });

  it("ping before the hello is answered; anything else first is Malformed", async () => {
    const r = room();
    const s = await r.open("join");
    await r.send(s, { type: "Ping" });
    expect(s.last).toEqual({ type: "Pong" });
    await r.send(s, { type: "Signal", peer: 1, signal: OFFER });
    expect(s.last).toMatchObject({ type: "Refused", reason: "Malformed" });
  });

  it("refuses frames meant for the other kind of socket", async () => {
    const r = room();
    const h = await r.open("host");
    await r.send(h, { type: "JoinHello", protocol: 1, door: DOOR, app: "x" });
    expect(h.last).toMatchObject({ type: "Refused", reason: "Malformed" });
    await r.host();
    const j = await r.join();
    await r.send(j, { type: "SetDoors", doors: [] });
    expect(j.last).toMatchObject({ type: "Refused", reason: "Malformed" });
    expect(r.core.state?.doors).toEqual([DOOR_HASH]);
    const k = await r.join();
    await r.send(k, { type: "JoinHello", protocol: 1, door: DOOR, app: "x" });
    expect(k.last).toMatchObject({ type: "Refused", reason: "Malformed" });
  });

  it("malformed and oversized frames close the socket", async () => {
    const r = room();
    const s = await r.open("join");
    await r.core.message(s, "x".repeat(LIMITS.maxFrameBytes + 1));
    expect(s.last).toMatchObject({ type: "Refused", reason: "Malformed" });
    expect(s.closedWith?.code).toBe(CLOSE.refused);
  });

  it(`token bucket: ${LIMITS.framesPerSecond} frames/s per socket`, async () => {
    const r = room();
    const s = await r.open("join");
    for (let i = 0; i < LIMITS.framesPerSecond; i++) await r.send(s, { type: "Ping" });
    expect(s.closedWith).toBeNull();
    await r.send(s, { type: "Ping" });
    expect(s.last).toMatchObject({ type: "Refused", reason: "RateLimited" });
    const t = await r.open("join");
    for (let i = 0; i < LIMITS.framesPerSecond; i++) await r.send(t, { type: "Ping" });
    r.clock.t += 100; // refills 5 tokens
    for (let i = 0; i < 5; i++) await r.send(t, { type: "Ping" });
    expect(t.closedWith).toBeNull();
  });

  it(`at most ${LIMITS.maxJoiners} joiner sockets per room`, async () => {
    const r = room();
    await r.host();
    for (let i = 0; i < LIMITS.maxJoiners; i++) expect((await r.join()).last?.type).toBe("JoinWelcome");
    const extra = await r.open("join");
    expect(extra.last).toMatchObject({ type: "Refused", reason: "RoomFull" });
  });

  it("pending joiner sockets count toward the room limit", async () => {
    const r = room();
    for (let i = 0; i < LIMITS.maxJoiners; i++) await r.open("join");
    expect((await r.open("join")).last).toMatchObject({ type: "Refused", reason: "RoomFull" });
    expect((await r.open("host")).closedWith).toBeNull();
  });

  it(`at most ${LIMITS.maxSignalsPerPeer} signals per pairing`, async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    for (let i = 0; i < LIMITS.maxSignalsPerPeer; i++) {
      r.clock.t += 100; // stay under the frame rate
      await r.send(i % 2 ? host : a, { type: "Signal", peer: 1, signal: OFFER });
    }
    expect(a.closedWith).toBeNull();
    r.clock.t += 100;
    await r.send(a, { type: "Signal", peer: 1, signal: OFFER });
    expect(a.last).toMatchObject({ type: "Refused", reason: "RateLimited" });
    expect(host.last).toEqual({ type: "PeerLeft", peer: 1 });
  });

  it(`at most ${LIMITS.badDoorsPerMinute} bad doors per IP per minute`, async () => {
    const r = room();
    await r.host();
    for (let i = 0; i < LIMITS.badDoorsPerMinute; i++) {
      expect((await r.join(BAD_DOOR, "6.6.6.6")).last).toMatchObject({ reason: "InvalidInvite" });
    }
    // Even the right door is refused now, from that address only.
    expect((await r.join(DOOR, "6.6.6.6")).last).toMatchObject({ type: "Refused", reason: "RateLimited" });
    expect((await r.join(DOOR, "7.7.7.7")).last?.type).toBe("JoinWelcome");
    r.clock.t += 60_000;
    expect((await r.join(DOOR, "6.6.6.6")).last?.type).toBe("JoinWelcome");
  });

  it(`at most ${LIMITS.claimsPerHour} room claims per IP per hour`, async () => {
    const budgets = new IpBudgets();
    const clock = { t: 1_700_000_000_000 };
    for (let i = 0; i < LIMITS.claimsPerHour; i++) {
      expect((await room({ budgets, clock }).host(TOKEN, [], "8.8.8.8")).last?.type).toBe("HostWelcome");
    }
    const r = room({ budgets, clock });
    expect((await r.host(TOKEN, [], "8.8.8.8")).last).toMatchObject({ type: "Refused", reason: "RateLimited" });
    expect(r.core.state).toBeNull();
    // Reconnecting to an existing room is not a claim.
    const own = room({ budgets, clock, record: { hostTokenHash: await sha256Hex(TOKEN), doors: [], lastHostSeenMs: clock.t } });
    expect((await own.host(TOKEN, [], "8.8.8.8")).last?.type).toBe("HostWelcome");
    clock.t += 3_600_000;
    expect((await room({ budgets, clock }).host(TOKEN, [], "8.8.8.8")).last?.type).toBe("HostWelcome");
  });
});

describe("hibernation", () => {
  it("a core rebuilt from the record and the socket states carries on", async () => {
    const r = room();
    const host = await r.host();
    const a = await r.join();
    await r.core.closed(await r.join()); // peer 2 gone
    const record = r.core.state;
    // The Durable Object was evicted: same sockets (states from attachments), new core.
    const again = room({ record, clock: r.clock, sockets: [host, a] });
    await again.send(a, { type: "Signal", peer: 0, signal: OFFER });
    expect(host.last).toEqual({ type: "Signal", peer: 1, signal: OFFER });
    const b = await again.join();
    expect(b.last).toMatchObject({ type: "JoinWelcome", peer: 2 });
    await again.core.closed(host);
    expect(a.last?.type).toBe("HostOffline");
    expect(again.core.state?.lastHostSeenMs).toBe(r.clock.t);
  });
});
