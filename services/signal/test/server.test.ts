import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { onRequest } from "../../../scripts/release/web/functions/signal/[[path]].ts";
import { startSignalServer, type SignalServer } from "./server.ts";
import { SmokeClient, smoke } from "./smoke.ts";

const ROOM = "AbCdEfGhIjKlMnOpQrStUv";

describe("Node adapter", () => {
  let server: SignalServer;
  beforeAll(async () => {
    server = await startSignalServer({ port: 0, allowedOrigins: "http://localhost:*" });
  });
  afterAll(async () => {
    await server.close();
  });

  it("passes the smoke test (host, joiners, signals, offline wait, CloseRoom)", async () => {
    await smoke(server.url);
  });

  it("serves the same routes under /signal", async () => {
    await smoke(`${server.url}/signal`);
  });

  it("forgets closed rooms", async () => {
    // Both smoke rooms ended with CloseRoom; wait for the last close events.
    await expect.poll(() => server.roomCount()).toBe(0);
  });

  it("answers health with CORS for allowed origins only", async () => {
    const ok = await fetch(`${server.url}/v1/health`, { headers: { Origin: "http://localhost:5173" } });
    expect(await ok.json()).toEqual({ ok: true, protocol: 1 });
    expect(ok.headers.get("access-control-allow-origin")).toBe("http://localhost:5173");
    expect((await fetch(`${server.url}/v1/health`, { headers: { Origin: "https://evil.example" } })).status).toBe(403);
    expect((await fetch(`${server.url}/v1/nope`)).status).toBe(404);
  });

  it("refuses socket upgrades from other browser origins", async () => {
    const { WebSocket: NodeWs } = await import("ws");
    const ws = new NodeWs(`${server.url.replace("http", "ws")}/v1/rooms/${ROOM}/join`, { headers: { Origin: "https://evil.example" } });
    const status = await new Promise<number>((resolve) => ws.on("unexpected-response", (_req, res) => resolve(res.statusCode ?? 0)));
    expect(status).toBe(403);
  });

  it("closes a socket that sends garbage", async () => {
    const c = new SmokeClient(`${server.url.replace("http", "ws")}/v1/rooms/${ROOM}/join`);
    await c.opened;
    (c as unknown as { ws: WebSocket }).ws.send("not json");
    expect((await c.next("Refused")).reason).toBe("Malformed");
    expect((await c.closed).code).toBe(4000);
  });
});

describe("Pages Function proxy", () => {
  const calls: { name: string; url: string }[] = [];
  const env = {
    ROOMS: {
      idFromName: (name: string) => name,
      get: (id: unknown) => ({
        fetch: async (r: Request) => {
          calls.push({ name: String(id), url: r.url });
          return new Response("from the room");
        },
      }),
    },
  };
  const call = (path: string, withEnv = env) => onRequest({ request: new Request(`https://etherealws.pages.dev${path}`), env: withEnv });

  it("forwards room routes to the room's Durable Object", async () => {
    expect(await (await call(`/signal/v1/rooms/${ROOM}/join`)).text()).toBe("from the room");
    expect(calls).toEqual([{ name: ROOM, url: `https://etherealws.pages.dev/signal/v1/rooms/${ROOM}/join` }]);
  });

  it("answers health itself and 404s the rest", async () => {
    expect(await (await call("/signal/v1/health")).json()).toEqual({ ok: true, protocol: 1 });
    expect((await call("/signal/v1/rooms/short/join")).status).toBe(404);
    expect((await call(`/signal/v1/rooms/${ROOM}/other`)).status).toBe(404);
  });

  it("answers 503 without the binding", async () => {
    expect((await call("/signal/v1/health", {} as typeof env)).status).toBe(503);
    expect((await call(`/signal/v1/rooms/${ROOM}/host`, {} as typeof env)).status).toBe(503);
  });
});
