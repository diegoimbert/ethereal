import { monotonicFactory } from "ulid";
import type { GestureId, ProjectId } from "@/generated";

const ulid = monotonicFactory();

/**
 * A new entity id (26-char ULID). The UI generates ids for everything it creates and
 * passes them in the command (`Track::Create { id: newId(), ... }`). Monotonic, so ids
 * created in a burst sort in creation order.
 */
export function newId(): string {
  return ulid();
}

/** Fills a byte array with random bytes. */
export type RandomBytes = (bytes: Uint8Array<ArrayBuffer>) => void;

const cryptoRandom: RandomBytes = (bytes) => {
  crypto.getRandomValues(bytes);
};

/**
 * A UUIDv7 (RFC 9562): 48-bit Unix-ms timestamp, version 7, 74 random bits, variant `10`.
 * Formatted lowercase `xxxxxxxx-xxxx-7xxx-[89ab]xxx-xxxxxxxxxxxx`. `now`/`random` are
 * injectable for tests and deterministic fixtures.
 */
export function uuidv7(now: number = Date.now(), random: RandomBytes = cryptoRandom): string {
  const b = new Uint8Array(16);
  random(b);
  let ts = Math.max(0, Math.floor(now));
  for (let i = 5; i >= 0; i--) {
    b[i] = ts % 256;
    ts = Math.floor(ts / 256);
  }
  b[6] = 0x70 | (b[6]! & 0x0f);
  b[8] = 0x80 | (b[8]! & 0x3f);
  const hex = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** A new project id (UUIDv7; projects are addressed by it in the engine-side store). */
export function newProjectId(): ProjectId {
  return uuidv7();
}

let lastGesture = 0;

/** A fresh gesture id (unique for this page; the engine only compares them for equality). */
export function nextGestureId(): GestureId {
  lastGesture = (lastGesture + 1) >>> 0 || 1;
  return lastGesture;
}

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const M64 = (1n << 64n) - 1n;

function decodeUlid(s: string): bigint {
  let v = 0n;
  for (const ch of s.toUpperCase()) {
    const d = CROCKFORD.indexOf(ch);
    if (d < 0) throw new Error(`not a ULID: ${s}`);
    v = (v << 5n) | BigInt(d);
  }
  return v & ((1n << 128n) - 1n);
}

function encodeUlid(v: bigint): string {
  let out = "";
  for (let i = 0; i < 26; i++) {
    out = CROCKFORD[Number(v & 31n)]! + out;
    v >>= 5n;
  }
  return out;
}

const rotl64 = (x: bigint, k: bigint) => ((x << k) | (x >> (64n - k))) & M64;

/** `ether_model::derive_id(seed, index)`: a deterministic id derived from a seed id. */
export function deriveId(seed: string, index: number): string {
  const u = decodeUlid(seed);
  const timestamp = u >> 80n;
  const random = u & ((1n << 80n) - 1n);
  const lo = random & M64;
  const hi = random >> 64n;
  const i = BigInt(index);
  let x = lo ^ rotl64(hi & M64, 17n) ^ (((i + 1n) * 0x9e3779b97f4a7c15n) & M64);
  x ^= x >> 30n;
  x = (x * 0xbf58476d1ce4e5b9n) & M64;
  x ^= x >> 27n;
  x = (x * 0x94d049bb133111ebn) & M64;
  x ^= x >> 31n;
  const h = (hi ^ rotl64(i, 32n)) & 0xffffn;
  const mixed = (h << 64n) | x;
  return encodeUlid((timestamp << 80n) | mixed);
}
