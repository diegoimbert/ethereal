/**
 * WasmTransport — browser host transport (controller in a Worker, engine in an
 * AudioWorklet, SharedArrayBuffer rings for playhead/meters).
 *
 * OWNERSHIP: the `wasm-host` node owns this folder (`ui/src/transport/wasm/**`).
 * Stub: `connect`/`send` reject with "not implemented" until that node implements it
 * (subscriptions are inert no-ops so the UI can render its connection-error state). Must honour the
 * `EngineTransport` contract (patches before replies, see `../EngineTransport.ts` and the
 * reference implementation `../mock/MockTransport.ts`).
 */

import type { Command, Event, MeterFrame, PlayheadFrame, Project, ReplyValue } from "@/generated";
import type { EngineTransport, SendOptions, Unsubscribe } from "../EngineTransport";

const NOT_IMPLEMENTED = "not implemented: wasm-host node";

export class WasmTransport implements EngineTransport {
  /** Flip to `true` once implemented (informational; selected via `VITE_ETHER_TRANSPORT=wasm`). */
  static readonly implemented: boolean = false;

  readonly kind = "wasm" as const;

  connect(): Promise<Project> {
    return Promise.reject(new Error(NOT_IMPLEMENTED));
  }
  send(_command: Command, _opts?: SendOptions): Promise<ReplyValue> {
    return Promise.reject(new Error(NOT_IMPLEMENTED));
  }
  onEvent(_listener: (event: Event) => void): Unsubscribe {
    return () => {};
  }
  subscribePlayhead(_listener: (frame: PlayheadFrame) => void): Unsubscribe {
    return () => {};
  }
  subscribeMeters(_listener: (frame: MeterFrame) => void): Unsubscribe {
    return () => {};
  }
  dispose(): void {}
}
