// The sync-FS wire protocol with a real second thread: the responder runs in a Node worker
// thread (as the OPFS Worker does in the browser) while this thread blocks in `collect`,
// like the controller Worker.
//
// No test depends on relative thread timing: the responder thread is running before any
// `collect` starts (so worker start-up under CPU load doesn't eat a reply timeout), and a
// "late" reply is held back by an explicit gate that this thread opens only after its call
// has timed out (a sleep could elapse first if this thread is descheduled under load).
import { Worker } from "node:worker_threads";
import { afterEach, describe, expect, it } from "vitest";
import { collect, FS_DONE, FS_ERROR, FsTimeoutError, resetForRequest, respond } from "./fsWire";

const HEADER = 16;
const CHUNK = 1024 * 1024; // same chunk area as the app (FS_CHUNK_BYTES)

interface Job {
  seq: number;
  /** Payload length; byte i is `(i * 31 + seq) & 0xff`. */
  len: number;
  final?: number;
  /** Wait until the test opens the gate (`openGate`) before responding. */
  gated?: boolean;
}

const pattern = (seq: number, len: number) => {
  const b = new Uint8Array(len);
  for (let i = 0; i < len; i++) b[i] = (i * 31 + seq) & 0xff;
  return b;
};

/**
 * Byte-exact comparison that stays O(n) native work. `expect(a).toEqual(b)` walks a typed
 * array element by element through vitest's generic deep equality, which costs seconds
 * for a multi-MB buffer (15 s measured under CPU load) and made the large-reply test time
 * out; this reports the length and the first differing offset instead.
 */
function expectBytes(actual: Uint8Array, expected: Uint8Array) {
  expect(actual.length).toBe(expected.length);
  const same = Buffer.from(actual.buffer, actual.byteOffset, actual.byteLength).equals(
    Buffer.from(expected.buffer, expected.byteOffset, expected.byteLength),
  );
  if (!same) {
    let i = 0;
    while (actual[i] === expected[i]) i++;
    expect({ firstMismatchAt: i, got: actual[i], want: expected[i] }).toBeUndefined();
  }
}

const workers: Worker[] = [];
afterEach(async () => {
  await Promise.all(workers.splice(0).map((w) => w.terminate()));
});

/**
 * Start a responder thread that answers `jobs` in order; resolves once the thread runs.
 * `openGate` releases the `gated` jobs.
 */
async function responder(
  buffer: SharedArrayBuffer,
  jobs: Job[],
  ackTimeoutMs = 5_000,
): Promise<{ openGate: () => void }> {
  const gate = new Int32Array(new SharedArrayBuffer(4));
  const src = `
    const { workerData, parentPort } = require("node:worker_threads");
    const respond = ${respond.toString()};
    const pattern = ${pattern.toString()};
    const { buffer, jobs, ackTimeoutMs, gateBuffer } = workerData;
    const ctrl = new Int32Array(buffer, 0, 4);
    const data = new Uint8Array(buffer, ${HEADER});
    const gate = new Int32Array(gateBuffer);
    parentPort.postMessage("ready");
    for (const job of jobs) {
      if (job.gated) while (Atomics.load(gate, 0) === 0) Atomics.wait(gate, 0, 0);
      respond(ctrl, data, job.seq, pattern(job.seq, job.len), job.final ?? ${FS_DONE}, ackTimeoutMs);
    }
  `;
  const w = new Worker(src, {
    eval: true,
    workerData: { buffer, jobs, ackTimeoutMs, gateBuffer: gate.buffer },
  });
  workers.push(w);
  await new Promise<void>((resolve, reject) => {
    w.once("message", () => resolve());
    w.once("error", reject);
  });
  return {
    openGate: () => {
      Atomics.store(gate, 0, 1);
      Atomics.notify(gate, 0);
    },
  };
}

function shared() {
  const buffer = new SharedArrayBuffer(HEADER + CHUNK);
  return { buffer, ctrl: new Int32Array(buffer, 0, 4), data: new Uint8Array(buffer, HEADER) };
}

describe("sync-FS wire protocol", () => {
  it("reassembles a reply larger than the chunk area (> 1 MB)", async () => {
    const { buffer, ctrl, data } = shared();
    const len = 3 * CHUNK + 12_345;
    resetForRequest(ctrl);
    await responder(buffer, [{ seq: 7, len }]);
    const reply = collect(ctrl, data, 7, 10_000);
    expect(reply.ok).toBe(true);
    if (reply.ok) expectBytes(reply.bytes, pattern(7, len));
  });

  it("returns exactly-one-chunk and empty replies", async () => {
    const { buffer, ctrl, data } = shared();
    resetForRequest(ctrl);
    await responder(buffer, [
      { seq: 1, len: CHUNK },
      { seq: 2, len: 0 },
    ]);
    const a = collect(ctrl, data, 1, 10_000);
    expect(a.ok).toBe(true);
    if (a.ok) expectBytes(a.bytes, pattern(1, CHUNK));
    const b = collect(ctrl, data, 2, 10_000);
    expect(b.ok && b.bytes.length).toBe(0);
  });

  it("delivers errors", async () => {
    const { buffer, ctrl, data } = shared();
    resetForRequest(ctrl);
    await responder(buffer, [{ seq: 3, len: 10, final: FS_ERROR }]);
    const reply = collect(ctrl, data, 3, 10_000);
    expect(reply).toEqual({ ok: false, error: pattern(3, 10) });
  });

  it("times out without a responder", () => {
    const { ctrl, data } = shared();
    resetForRequest(ctrl);
    expect(() => collect(ctrl, data, 1, 50)).toThrow(FsTimeoutError);
  });

  it("a late multi-chunk reply to a timed-out call never reaches the next call", async () => {
    const { buffer, ctrl, data } = shared();
    resetForRequest(ctrl);
    // Request 1 is answered late (only after the caller gave up), with a 2.5 MB reply;
    // then request 2 is answered normally.
    const { openGate } = await responder(buffer, [
      { seq: 1, len: 2 * CHUNK + CHUNK / 2, gated: true },
      { seq: 2, len: 1000 },
    ]);
    expect(() => collect(ctrl, data, 1, 50)).toThrow(FsTimeoutError);
    resetForRequest(ctrl);
    openGate();
    const reply = collect(ctrl, data, 2, 10_000);
    expect(reply.ok).toBe(true);
    if (reply.ok) expectBytes(reply.bytes, pattern(2, 1000));
  });
});
