// Synchronous file system for the Rust controller (`JsFsHost` in crates/ether-wasm), used
// inside the controller Worker. Each call posts an `FsRequest` to the OPFS Worker and blocks
// with `Atomics.wait` until the answer is in the shared fs buffer (chunked for big files).
import type { JsFsHost } from "@ether-wasm/ether_wasm.js";
import { FS_CHUNK, FS_ERROR, FS_HEADER_BYTES, FS_IDLE, type FsRequest } from "./protocol";

/** A stuck OPFS worker must not hang the controller forever. */
const TIMEOUT_MS = 30_000;
const decoder = new TextDecoder();

export class SyncFs implements JsFsHost {
  private readonly ctrl: Int32Array;
  private readonly data: Uint8Array;

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

  private call(req: FsRequest, transfer: Transferable[] = []): Uint8Array {
    Atomics.store(this.ctrl, 0, FS_IDLE);
    this.port.postMessage(req, transfer);
    const chunks: Uint8Array[] = [];
    for (;;) {
      if (Atomics.wait(this.ctrl, 0, FS_IDLE, TIMEOUT_MS) === "timed-out") {
        throw { code: "Io", message: `OPFS ${req.op} ${req.path}: timed out` };
      }
      const state = Atomics.load(this.ctrl, 0);
      const chunk = this.data.slice(0, Atomics.load(this.ctrl, 1));
      Atomics.store(this.ctrl, 0, FS_IDLE);
      Atomics.notify(this.ctrl, 0);
      if (state === FS_ERROR) throw JSON.parse(decoder.decode(chunk)) as { code: string; message: string };
      chunks.push(chunk);
      if (state !== FS_CHUNK) break;
    }
    if (chunks.length === 1) return chunks[0]!;
    const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
    let off = 0;
    for (const c of chunks) {
      out.set(c, off);
      off += c.length;
    }
    return out;
  }
}
