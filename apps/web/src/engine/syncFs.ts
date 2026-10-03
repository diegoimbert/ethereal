// Synchronous file system for the Rust controller (`JsFsHost` in crates/ether-wasm), used
// inside the controller Worker. Each call posts an `FsRequest` to the OPFS Worker and blocks
// with `Atomics.wait` until the answer is in the shared fs buffer (see ./fsWire.ts).
import type { JsFsHost } from "@ether-wasm/ether_wasm.js";
import { collect, FsTimeoutError, resetForRequest } from "./fsWire";
import { FS_HEADER_BYTES, type FsRequest } from "./protocol";

/** A stuck OPFS worker must not hang the controller forever. */
const TIMEOUT_MS = 30_000;
const decoder = new TextDecoder();

export class SyncFs implements JsFsHost {
  private readonly ctrl: Int32Array;
  private readonly data: Uint8Array;
  private seq = 0;

  constructor(
    buffer: SharedArrayBuffer,
    private readonly port: MessagePort,
  ) {
    this.ctrl = new Int32Array(buffer, 0, FS_HEADER_BYTES / 4);
    this.data = new Uint8Array(buffer, FS_HEADER_BYTES);
  }

  read(path: string): Uint8Array {
    return this.call({ op: "read", path });
  }
  write(path: string, bytes: Uint8Array): void {
    // `bytes` views wasm memory: copy before transferring.
    const copy = bytes.slice();
    this.call({ op: "write", path, bytes: copy }, [copy.buffer]);
  }
  rename(from: string, to: string): void {
    this.call({ op: "rename", path: from, to });
  }
  list(dir: string): string {
    return decoder.decode(this.call({ op: "list", path: dir }));
  }
  mkdir(path: string): void {
    this.call({ op: "mkdir", path });
  }
  remove(path: string): void {
    this.call({ op: "remove", path });
  }
  stat(path: string): string {
    return decoder.decode(this.call({ op: "stat", path }));
  }
  readRange(path: string, offset: number, length: number): Uint8Array {
    return this.call({ op: "readRange", path, offset, length });
  }

  private call(req: Omit<FsRequest, "seq">, transfer: Transferable[] = []): Uint8Array {
    this.seq = (this.seq + 1) | 0;
    const seq = this.seq;
    resetForRequest(this.ctrl);
    this.port.postMessage({ ...req, seq } satisfies FsRequest, transfer);
    let reply;
    try {
      reply = collect(this.ctrl, this.data, seq, TIMEOUT_MS);
    } catch (e) {
      if (e instanceof FsTimeoutError) throw { code: "Io", message: `OPFS ${req.op} ${req.path}: timed out` };
      throw e;
    }
    if (!reply.ok) throw JSON.parse(decoder.decode(reply.error)) as { code: string; message: string };
    return reply.bytes;
  }
}
