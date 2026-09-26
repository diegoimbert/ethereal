// Wire protocol of the sync file system: how the OPFS Worker hands a reply (of any size)
// to the controller Worker, which is blocked in `Atomics.wait`.
//
// Shared buffer: Int32 `ctrl` slots [0] state, [1] chunk length, [2] sequence number,
// then the chunk area `data`. Every frame the responder publishes (a chunk, the last
// chunk, or an error) is acknowledged by the caller setting the state back to IDLE; the
// responder never writes the next frame before that ack (or an ack timeout), so a frame is
// never overwritten while it is being read. Each request carries a sequence number that
// the responder echoes in ctrl[2]: frames of an older request (one the caller gave up on
// after a timeout) are acknowledged and discarded instead of being taken as the answer.
//
// `respond` must stay self-contained (no imports or module constants): the unit test runs
// it in a Node worker thread from its source text.

/** Buffer states (ctrl[0]). */
export const FS_IDLE = 0;
/** A chunk is in the buffer; more follow. */
export const FS_CHUNK = 1;
/** The last chunk is in the buffer. */
export const FS_DONE = 2;
/** The buffer holds a JSON `{ code, message }` error. */
export const FS_ERROR = 3;

/**
 * Responder side (OPFS Worker): publish `bytes` for request `seq` in chunks of
 * `data.length`, ending with state `final` (FS_DONE or FS_ERROR). Returns `false` if the
 * caller stopped acknowledging (it timed out and went away).
 */
export function respond(
  ctrl: Int32Array,
  data: Uint8Array,
  seq: number,
  bytes: Uint8Array,
  final: number,
  ackTimeoutMs: number,
): boolean {
  const CHUNK = 1;
  let off = 0;
  for (;;) {
    const n = Math.min(data.length, bytes.length - off);
    data.set(bytes.subarray(off, off + n));
    off += n;
    const state = off >= bytes.length ? final : CHUNK;
    Atomics.store(ctrl, 1, n);
    Atomics.store(ctrl, 2, seq);
    Atomics.store(ctrl, 0, state);
    Atomics.notify(ctrl, 0);
    // Wait for the ack (state back to IDLE) before touching the buffer again.
    if (Atomics.wait(ctrl, 0, state, ackTimeoutMs) === "timed-out") return false;
    if (state !== CHUNK) return true;
  }
}

export type Reply = { ok: true; bytes: Uint8Array } | { ok: false; error: Uint8Array };

export class FsTimeoutError extends Error {}

/**
 * Caller side (controller Worker): block until the reply to request `seq` is complete.
 * Frames of other requests are acknowledged and dropped. Throws `FsTimeoutError`.
 */
export function collect(ctrl: Int32Array, data: Uint8Array, seq: number, timeoutMs: number): Reply {
  const deadline = Date.now() + timeoutMs;
  const chunks: Uint8Array[] = [];
  for (;;) {
    const left = deadline - Date.now();
    if (left <= 0 || Atomics.wait(ctrl, 0, FS_IDLE, left) === "timed-out") {
      throw new FsTimeoutError(`no reply within ${timeoutMs} ms`);
    }
    const state = Atomics.load(ctrl, 0);
    const chunk = data.slice(0, Atomics.load(ctrl, 1));
    const from = Atomics.load(ctrl, 2);
    Atomics.store(ctrl, 0, FS_IDLE);
    Atomics.notify(ctrl, 0);
    if (from !== seq) continue;
    if (state === FS_ERROR) return { ok: false, error: chunk };
    chunks.push(chunk);
    if (state !== FS_CHUNK) break;
  }
  if (chunks.length === 1) return { ok: true, bytes: chunks[0]! };
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.length;
  }
  return { ok: true, bytes: out };
}

/** Before sending a new request: release a responder still waiting on a stale frame. */
export function resetForRequest(ctrl: Int32Array): void {
  Atomics.store(ctrl, 0, FS_IDLE);
  Atomics.notify(ctrl, 0);
}
