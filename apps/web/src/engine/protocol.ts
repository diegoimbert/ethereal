// Messages between the main thread, the controller Worker and the OPFS Worker, plus the
// shared-buffer sizes. (Worker ↔ Worklet traffic goes through the SAB rings, in Rust.)

/** Ring layout: 16 header bytes + a power-of-two data area (crates/ether-wasm/src/ring.rs). */
export const RING_HEADER_BYTES = 16;
/** Control ring (graph snapshots, params, transport, decoded media), Worker → Worklet. */
export const CONTROL_RING_BYTES = 4 * 1024 * 1024;
/** Report ring (playhead, meters, errors), Worklet → Worker. */
export const REPORT_RING_BYTES = 256 * 1024;
/** Sync-FS response buffer: 16 header bytes + chunk area. */
export const FS_HEADER_BYTES = 16;
export const FS_CHUNK_BYTES = 1024 * 1024;

export function ringBuffer(dataBytes: number): SharedArrayBuffer {
  return new SharedArrayBuffer(RING_HEADER_BYTES + dataBytes);
}

/** `mode` of the Rust controller: the real `EtherController`, or the smoke-test fake. */
export type ControllerMode = "ether" | "fake";

/** Main → controller Worker. */
export type ToController =
  | {
      type: "init";
      module: WebAssembly.Module;
      control: SharedArrayBuffer;
      reports: SharedArrayBuffer;
      fsBuffer: SharedArrayBuffer;
      fsPort: MessagePort;
      seed: string;
      mode: ControllerMode;
    }
  | { type: "client"; json: string };

/** Controller Worker → main. */
export type FromController =
  | { type: "ready" }
  | { type: "server"; json: string }
  | { type: "fatal"; message: string };

/** Sync-FS request, controller Worker → OPFS Worker (answered through the fs buffer). */
export type FsOp = "read" | "write" | "rename" | "list" | "mkdir" | "remove" | "stat";
export interface FsRequest {
  op: FsOp;
  path: string;
  /** `rename` destination. */
  to?: string;
  /** `write` payload. */
  bytes?: Uint8Array;
}

/** Sync-FS buffer states (Int32 slot 0). */
export const FS_IDLE = 0;
/** A chunk of the response is in the buffer; more follow once the reader acks (→ IDLE). */
export const FS_CHUNK = 1;
/** The last chunk is in the buffer. */
export const FS_DONE = 2;
/** The buffer holds a JSON `{ code, message }` error. */
export const FS_ERROR = 3;
