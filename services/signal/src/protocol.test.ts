import { describe, expect, it } from "vitest";
import { originAllowed, parseClientMessage, route, sha256Hex } from "./protocol.ts";

const ROOM = "AbCdEfGhIjKlMnOpQrStUv";
const HASH = "a".repeat(64);
const TOKEN = "t".repeat(43);

describe("routes", () => {
  it("maps health, host and join, also under /signal", () => {
    expect(route("/v1/health")).toEqual({ kind: "health" });
    expect(route(`/v1/rooms/${ROOM}/host`)).toEqual({ kind: "host", room: ROOM });
    expect(route(`/signal/v1/rooms/${ROOM}/join`)).toEqual({ kind: "join", room: ROOM });
    expect(route("/v1/rooms/short/join")).toBeNull();
    expect(route(`/v1/rooms/${ROOM}/other`)).toBeNull();
  });
});

describe("origins", () => {
  const allowed = "https://etherealws.pages.dev,https://*.etherealws.pages.dev,http://localhost:*";
  it("allows native clients, the app and its previews", () => {
    expect(originAllowed(null, allowed)).toBe(true);
    expect(originAllowed("https://etherealws.pages.dev", allowed)).toBe(true);
    expect(originAllowed("https://abc123.etherealws.pages.dev", allowed)).toBe(true);
    expect(originAllowed("http://localhost:5173", allowed)).toBe(true);
    expect(originAllowed("https://evil.example", allowed)).toBe(false);
    expect(originAllowed("https://etherealws.pages.dev.evil.example", allowed)).toBe(false);
  });
});

describe("client messages", () => {
  it("accepts well-formed frames", () => {
    expect(parseClientMessage(JSON.stringify({ type: "HostHello", protocol: 1, host_token: TOKEN, doors: [HASH], app: "Ethereal 0.3" }))).not.toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "JoinHello", protocol: 1, door: ROOM, app: "x" }))).not.toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "Signal", peer: 3, signal: { type: "Offer", sdp: "v=0" } }))).not.toBeNull();
    const ice = { type: "Ice", candidate: { candidate: "candidate:1 1 udp 1 1.2.3.4 5 typ host", sdp_mid: "0", sdp_m_line_index: 0, username_fragment: null } };
    expect(parseClientMessage(JSON.stringify({ type: "Signal", peer: 3, signal: ice }))).not.toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "Ping", extra: 1 }))).toEqual({ type: "Ping" });
  });

  it("refuses malformed or oversized frames", () => {
    expect(parseClientMessage(JSON.stringify({ type: "Ping", pad: "x".repeat(70_000) }))).toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "SetDoors", doors: Array(67).fill(HASH) }))).toBeNull();
    expect(parseClientMessage("nope")).toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "HostHello", protocol: 1, host_token: "short", doors: [], app: "" }))).toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "SetDoors", doors: ["not-hex"] }))).toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "Signal", peer: -1, signal: { type: "Bye", reason: null } }))).toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "Signal", peer: 1, signal: { type: "Offer", sdp: "x".repeat(40_000) } }))).toBeNull();
    expect(parseClientMessage(JSON.stringify({ type: "Unknown" }))).toBeNull();
  });

  it("hashes like the engine (lowercase hex SHA-256)", async () => {
    expect(await sha256Hex("abc")).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  });
});
