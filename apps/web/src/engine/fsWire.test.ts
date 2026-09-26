// The sync-FS wire protocol with a real second thread: the responder runs in a Node worker
// thread (as the OPFS Worker does in the browser) while this thread blocks in `collect`,
// like the controller Worker.
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
  /** Sleep before responding. */
  delayMs?: number;
}

const pattern = (seq: number, len: number) => {
  const b = new Uint8Array(len);
  for (let i = 0; i < len; i++) b[i] = (i * 31 + seq) & 0xff;
  return b;
};

const workers: Worker[] = [];
afterEach(async () => {
  await Promise.all(workers.splice(0).map((w) => w.terminate()));
});

/** Start a responder thread that answers `jobs` in order. */
function responder(buffer: SharedArrayBuffer, jobs: Job[], ackTimeoutMs = 5_000): Worker {
  const src = `
    const { workerData } = require("node:worker_threads");
    const respond = ${respond.toString()};
    const pattern = ${pattern.toString()};
    const { buffer, jobs, ackTimeoutMs } = workerData;
    const ctrl = new Int32Array(buffer, 0, 4);
    const data = new Uint8Array(buffer, ${HEADER});
    const sleep = new Int32Array(new SharedArrayBuffer(4));
    for (const job of jobs) {
      if (job.delayMs) Atomics.wait(sleep, 0, 0, job.delayMs);
      respond(ctrl, data, job.seq, pattern(job.seq, job.len), job.final ?? ${FS_DONE}, ackTimeoutMs);
    }
  `;
  const w = new Worker(src, { eval: true, workerData: { buffer, jobs, ackTimeoutMs } });
  workers.push(w);
  return w;
}

function shared() {
  const buffer = new SharedArrayBuffer(HEADER + CHUNK);
  return { buffer, ctrl: new Int32Array(buffer, 0, 4), data: new Uint8Array(buffer, HEADER) };
}

describe("sync-FS wire protocol", () => {
  it("reassembles a reply larger than the chunk area (> 1 MB)", () => {
    const { buffer, ctrl, data } = shared();
    const len = 3 * CHUNK + 12_345;
    resetForRequest(ctrl);
    responder(buffer, [{ seq: 7, len }]);
    const reply = collect(ctrl, data, 7, 10_000);
    expect(reply.ok).toBe(true);
    if (reply.ok) expect(reply.bytes).toEqual(pattern(7, len));
  });

  it("returns exactly-one-chunk and empty replies", () => {
    const { buffer, ctrl, data } = shared();
    resetForRequest(ctrl);
    responder(buffer, [
      { seq: 1, len: CHUNK },
      { seq: 2, len: 0 },
    ]);
    const a = collect(ctrl, data, 1, 10_000);
    expect(a.ok && a.bytes.length).toBe(CHUNK);
    const b = collect(ctrl, data, 2, 10_000);
    expect(b.ok && b.bytes.length).toBe(0);
  });

  it("delivers errors", () => {
    const { buffer, ctrl, data } = shared();
    resetForRequest(ctrl);
    responder(buffer, [{ seq: 3, len: 10, final: FS_ERROR }]);
    const reply = collect(ctrl, data, 3, 10_000);
    expect(reply).toEqual({ ok: false, error: pattern(3, 10) });
  });

  it("times out without a responder", () => {
    const { ctrl, data } = shared();
    resetForRequest(ctrl);
    expect(() => collect(ctrl, data, 1, 50)).toThrow(FsTimeoutError);
  });

  it("a late multi-chunk reply to a timed-out call never reaches the next call", () => {
    const { buffer, ctrl, data } = shared();
    resetForRequest(ctrl);
    // Request 1 is answered late (after the caller gave up), with a 2.5 MB reply; then
    // request 2 is answered normally.
    responder(buffer, [
      { seq: 1, len: 2 * CHUNK + CHUNK / 2, delayMs: 300 },
      { seq: 2, len: 1000 },
    ]);
    expect(() => collect(ctrl, data, 1, 50)).toThrow(FsTimeoutError);
    resetForRequest(ctrl);
    const reply = collect(ctrl, data, 2, 10_000);
    expect(reply.ok).toBe(true);
    if (reply.ok) expect(reply.bytes).toEqual(pattern(2, 1000));
  });
});
