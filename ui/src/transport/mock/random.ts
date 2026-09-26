import { monotonicFactory } from "ulid";
import { uuidv7 } from "../ids";

/** Small deterministic PRNG (mulberry32), returns floats in [0, 1). */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Fixed timestamp used for deterministic ids (2026-01-01T00:00:00Z). */
export const SEED_TIME = 1767225600000;

/** A deterministic UUIDv7 project id for `seed` (timestamp `SEED_TIME + seed` ms). */
export function seededProjectId(seed: number): string {
  const rand = mulberry32(seed ^ 0x5eed);
  return uuidv7(SEED_TIME + seed, (bytes) => {
    for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(rand() * 256);
  });
}

/**
 * A deterministic ULID generator: same seed → same sequence of valid, increasing ids.
 * Used for the demo project and in tests.
 */
export function seededIdFactory(seed: number): () => string {
  const factory = monotonicFactory(mulberry32(seed));
  return () => factory(SEED_TIME);
}
