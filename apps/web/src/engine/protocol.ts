// Messages between the main thread, the controller Worker and the OPFS Worker, plus the
// shared-buffer sizes. (Worker ↔ Worklet traffic goes through the SAB rings, in Rust.)

import type { ModuleInfo } from "./webclap/wasm";

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
       * One Int32: how many `client` messages the Worker has handled (a closing page
       * waits for its last save, `WasmEndpoint.drain`).
       */
      handled: SharedArrayBuffer;
      /** The installed WebCLAP plugins (JSON list of scanned modules) and their modules. */
      webclapCatalog?: string;
      webclapModules?: Omit<WebclapModuleMsg, "type">[];
      /** An extension's own init fields (`WebEngineExtension.init`). */
      [extra: string]: unknown;
    }
  | { type: "client"; json: string }
  /** WebCLAP plugins installed or removed: the new catalog, then any new module. */
  | { type: "webclap:catalog"; json: string }
  | WebclapModuleMsg
  /** An extension's own message (`ControllerWorkerExtension.onMessage`). */
  | { type: `ext:${string}`; [field: string]: unknown };

/** Controller Worker → main. */
export type FromController =
  | { type: "ready" }
  | { type: "server"; json: string }
  | { type: "fatal"; message: string }
  /** An extension's own message (`WebEngineExtension.onControllerMessage`). */
  | { type: `ext:${string}`; [field: string]: unknown };

/** An installed WebCLAP module as the engine realms get it (compiled once, on the page). */
export interface WebclapModuleMsg {
  type: "webclap:module";
  id: string;
  module: WebAssembly.Module;
  info: ModuleInfo;
}

/** Page ↔ Worklet messages of WebCLAP (modules, the plugin UIs), on the node's port. */
export type WebclapPortMsg =
  | WebclapModuleMsg
  /** UI → plugin (`receive`), or plugin → UI (`send`). */
  | { type: "webclap:ui"; device: string; data: ArrayBuffer }
  | { type: "webclap:open"; device: string; req: number }
  | { type: "webclap:opened"; req: number; uri?: string; error?: string }
  | { type: "webclap:close"; device: string }
  | { type: "webclap:resource"; device: string; path: string; req: number }
  | { type: "webclap:resourced"; req: number; mime?: string; data?: ArrayBuffer };

/** Sync-FS request, controller Worker → OPFS Worker (answered through the fs buffer). */
export type FsOp = "read" | "write" | "rename" | "list" | "mkdir" | "remove" | "stat" | "readRange";
export interface FsRequest {
  op: FsOp;
  path: string;
  /** `rename` destination. */
  to?: string;
  /** `write` payload. */
  bytes?: Uint8Array;
  /** `readRange`: byte offset and length (audio streaming reads media in chunks). */
  offset?: number;
  length?: number;
  /** Echoed in the reply (see ./fsWire.ts). */
  seq: number;
}
