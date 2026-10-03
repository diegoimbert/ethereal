import { describe, expect, it } from "vitest";
import { IpBudgets } from "./budget.ts";
import { IceProvider, mintTurn, stunServers, TURN_REUSE_MS } from "./ice.ts";

const STUN = "stun:stun.cloudflare.com:3478";

describe("ICE servers", () => {
  it("STUN_URLS becomes one IceServer (non-STUN entries ignored)", () => {
    expect(stunServers(`${STUN}, stun:other:19302 ,http://nope`)).toEqual([{ urls: [STUN, "stun:other:19302"], username: null, credential: null }]);
    expect(stunServers("")).toEqual([]);
    expect(stunServers(undefined)).toEqual([]);
  });

  const cfAnswer = {
    iceServers: [
      { urls: ["stun:stun.cloudflare.com:3478", "stun:stun.cloudflare.com:53"] },
      {
        urls: ["turn:turn.cloudflare.com:3478?transport=udp", "turn:turn.cloudflare.com:53?transport=udp", "turns:turn.cloudflare.com:443?transport=tcp"],
        username: "u",
        credential: "c",
      },
    ],
  };

  it("mints TURN credentials, keeps the TURN entry and drops port 53", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const servers = await mintTurn("key-id", "secret", async (url, init) => {
      calls.push({ url, init });
      return Response.json(cfAnswer);
    });
    expect(servers).toEqual([
      { urls: ["turn:turn.cloudflare.com:3478?transport=udp", "turns:turn.cloudflare.com:443?transport=tcp"], username: "u", credential: "c" },
    ]);
    expect(calls[0]?.url).toBe("https://rtc.live.cloudflare.com/v1/turn/keys/key-id/credentials/generate-ice-servers");
    expect((calls[0]?.init.headers as Record<string, string>).Authorization).toBe("Bearer secret");
  });

  it("accepts the older single-object answer", async () => {
    const one = { iceServers: { urls: ["turn:t:3478"], username: "u", credential: "c" } };
    expect(await mintTurn("k", "t", async () => Response.json(one))).toEqual([{ urls: ["turn:t:3478"], username: "u", credential: "c" }]);
  });

  it("falls back to STUN only when TURN fails", async () => {
    expect(await mintTurn("k", "t", async () => new Response("no", { status: 401 }))).toEqual([]);
    expect(
      await mintTurn("k", "t", async () => {
        throw new Error("offline");
      }),
    ).toEqual([]);
  });

  it("the provider caches TURN for TURN_REUSE_MS and retries failures at once", async () => {
    let now = 0;
    let calls = 0;
    let fail = true;
    const p = new IceProvider({ STUN_URLS: STUN, TURN_KEY_ID: "k", TURN_KEY_API_TOKEN: "t" }, async () => {
      calls++;
      return fail ? new Response("", { status: 500 }) : Response.json(cfAnswer);
    }, () => now);
    expect(await p.servers()).toHaveLength(1);
    fail = false;
    expect(await p.servers()).toHaveLength(2);
    expect(calls).toBe(2);
    now += TURN_REUSE_MS;
    await p.servers();
    expect(calls).toBe(2);
    now += 1;
    await p.servers();
    expect(calls).toBe(3);
    const stunOnly = new IceProvider({ STUN_URLS: STUN }, async () => {
      throw new Error("never called");
    }, () => 0);
    expect(await stunOnly.servers()).toEqual([{ urls: [STUN], username: null, credential: null }]);
  });
});

describe("per-IP budgets", () => {
  it("peek does not spend; windows slide; state round-trips through JSON", () => {
    const b = new IpBudgets();
    for (let i = 0; i < 19; i++) expect(b.check("badDoor", "ip", "spend", 1000)).toBe(true);
    expect(b.check("badDoor", "ip", "peek", 1000)).toBe(true);
    expect(b.check("badDoor", "ip", "spend", 1000)).toBe(true); // the 20th
    expect(b.check("badDoor", "ip", "peek", 1000)).toBe(false);
    expect(b.check("badDoor", "other", "peek", 1000)).toBe(true);
    expect(b.check("claim", "ip", "peek", 1000)).toBe(true);
    const restored = new IpBudgets(JSON.parse(JSON.stringify(b)) as Record<string, number[]>);
    expect(restored.check("badDoor", "ip", "peek", 1000)).toBe(false);
    expect(restored.check("badDoor", "ip", "peek", 61_000)).toBe(true);
    restored.sweep(61_000);
    expect(restored.toJSON()).toEqual({});
  });
});
