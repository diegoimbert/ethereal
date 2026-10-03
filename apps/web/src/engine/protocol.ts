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
      /** AudioContext sample rate (the engine's rate). */
      sampleRate: number;
      /**
       * The Worker's end of the share port (docs/SHARING.md §6.2): the UI's WebRTC peer
       * connections for `ether_collab::share::web` (UI agent: `@/features/share/endpoint`).
       */
      sharePort: MessagePort;
    }
  | { type: "client"; json: string }
  /** Diagnostics and e2e: call a `ShareProbe` method (crates/ether-wasm/src/share.rs). */
  | { type: "share-probe"; id: number; method: ShareProbeMethod; args: unknown[] };

/** The methods of `ShareProbe` (one probe per Worker, created on first use). */
export type ShareProbeMethod =
  | "open"
  | "signal"
  | "poll"
  | "send_text"
  | "send_pattern"
  | "recv"
  | "buffered"
  | "state"
  | "close";

/** Controller Worker → main. */
export type FromController =
  | { type: "ready" }
  | { type: "server"; json: string }
  | { type: "fatal"; message: string }
  /** Answer to a `share-probe` call (`error` when it threw). */
  | { type: "share-probe"; id: number; result?: unknown; error?: string };

/** Sync-FS request, controller Worker → OPFS Worker (answered through the fs buffer). */
export type FsOp = "read" | "write" | "rename" | "list" | "mkdir" | "remove" | "stat" | "readRange";
export interface FsRequest {
  op: FsOp;
  path: string;
  /** `rename` destination. */
  to?: string;
  /** `write` payload. */
  bytes?: Uint8Array;
  /** `readRange`: byte offset and length (audio-streaming reads media in chunks). */
  offset?: number;
  length?: number;
  /** Echoed in the reply (see ./fsWire.ts). */
  seq: number;
}
