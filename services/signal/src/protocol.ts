/**
 * Signaling wire (docs/SHARING.md §3.2). The message types are generated from Rust
 * (`ether_protocol::share`, `just gen-types`) and imported as types only, so the Worker
 * bundle stays tiny and the two sides can't drift.
 *
 * This file is the frozen part of the skeleton: routes, limits, and the validation every
 * inbound frame goes through before the room logic sees it.
 */

import type { SignalClientMessage, SignalRefusal, SignalServerMessage, StreamSignal } from "../../../ui/src/generated";

export type { SignalClientMessage, SignalRefusal, SignalServerMessage };

/** `ether_protocol::share::SIGNAL_PROTOCOL_VERSION`. */
export const SIGNAL_PROTOCOL_VERSION = 1;

export const LIMITS = {
  /** Largest inbound text frame (an SDP is ≤ 32 KiB, docs/COLLAB.md §9.7). */
  maxFrameBytes: 64 * 1024,
  maxSdpBytes: 32 * 1024,
  maxCandidateBytes: 1024,
  /** Doors per room (2 links + members). */
  maxDoors: 66,
  /** Joiner sockets per room (waiting for the host or paired). */
  maxJoiners: 32,
  /** Inbound frames per socket per second (token bucket, burst = rate). */
  framesPerSecond: 50,
  /** Signals per pairing (offer, answer, trickle ICE, restarts). */
  maxSignalsPerPeer: 200,
  /** The first frame must arrive within this. */
  helloTimeoutMs: 10_000,
  /** A joiner waits for an offline host at most this long, then is closed (it retries). */
  waitForHostMs: 10 * 60_000,
  /** Failed `JoinHello`s per client IP per minute before `RateLimited`. */
  badDoorsPerMinute: 20,
  /** Room claims (first `HostHello`) per client IP per hour. */
  claimsPerHour: 30,
} as const;

/** A room id or door: 22 base64url chars (16 bytes). */
const ID = /^[A-Za-z0-9_-]{22}$/;
/** A host token: 43 base64url chars (32 bytes). */
const TOKEN = /^[A-Za-z0-9_-]{43}$/;
/** SHA-256 in lowercase hex. */
const SHA256_HEX = /^[0-9a-f]{64}$/;

export type Route =
  | { kind: "health" }
  | { kind: "host"; room: string }
  | { kind: "join"; room: string };

/** `GET /v1/health`, `GET /v1/rooms/<room>/host` and `/join` (WebSocket upgrades). */
export function route(pathname: string): Route | null {
  const p = pathname.replace(/^\/signal(?=\/)/, ""); // mounted under /signal on Pages
  if (p === "/v1/health") return { kind: "health" };
  const m = /^\/v1\/rooms\/([^/]+)\/(host|join)$/.exec(p);
  if (!m || !ID.test(m[1] ?? "")) return null;
  return { kind: m[2] === "host" ? "host" : "join", room: m[1] ?? "" };
}

/** Browser origins allowed by `ALLOWED_ORIGINS` (no Origin header = a native client). */
export function originAllowed(origin: string | null, allowed: string): boolean {
  if (origin === null) return true;
  return allowed
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
    .some((pattern) => {
      if (pattern === origin) return true;
      const re = new RegExp(`^${pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*")}$`);
      return re.test(origin);
    });
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown, max = 256): v is string => typeof v === "string" && v.length <= max;
const peerId = (v: unknown): v is number => typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= 0xffff_ffff;

function validSignal(v: unknown): v is StreamSignal {
  if (!isObject(v)) return false;
  switch (v.type) {
    case "Offer":
    case "Answer":
      return str(v.sdp, LIMITS.maxSdpBytes);
    case "Ice": {
      const c = v.candidate;
      return (
        isObject(c) &&
        str(c.candidate, LIMITS.maxCandidateBytes) &&
        (c.sdp_mid === null || str(c.sdp_mid, 64)) &&
        (c.sdp_m_line_index === null || (typeof c.sdp_m_line_index === "number" && c.sdp_m_line_index >= 0 && c.sdp_m_line_index < 65536)) &&
        (c.username_fragment === null || str(c.username_fragment, 256))
      );
    }
    case "Bye":
      return v.reason === null || str(v.reason, LIMITS.maxCandidateBytes);
    default:
      return false;
  }
}

/** Parse and validate one inbound text frame; `null` = malformed (the socket is closed). */
export function parseClientMessage(text: string): SignalClientMessage | null {
  if (text.length > LIMITS.maxFrameBytes) return null;
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObject(v)) return null;
  switch (v.type) {
    case "HostHello":
      return typeof v.protocol === "number" &&
        str(v.host_token) &&
        TOKEN.test(v.host_token) &&
        Array.isArray(v.doors) &&
        v.doors.length <= LIMITS.maxDoors &&
        v.doors.every((d) => typeof d === "string" && SHA256_HEX.test(d)) &&
        str(v.app)
        ? (v as SignalClientMessage)
        : null;
    case "SetDoors":
      return Array.isArray(v.doors) && v.doors.length <= LIMITS.maxDoors && v.doors.every((d) => typeof d === "string" && SHA256_HEX.test(d))
        ? (v as SignalClientMessage)
        : null;
    case "CloseRoom":
    case "Ping":
      return { type: v.type };
    case "JoinHello":
      return typeof v.protocol === "number" && str(v.door) && ID.test(v.door) && str(v.app) ? (v as SignalClientMessage) : null;
    case "Signal":
      return peerId(v.peer) && validSignal(v.signal) ? (v as SignalClientMessage) : null;
    case "EndPeer":
      return peerId(v.peer) && (v.reason === null || str(v.reason, LIMITS.maxCandidateBytes)) ? (v as SignalClientMessage) : null;
    default:
      return null;
  }
}

export const refused = (reason: SignalRefusal, message: string): SignalServerMessage => ({ type: "Refused", reason, message });

/** Lowercase hex SHA-256 (doors and host tokens are stored hashed). */
export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
