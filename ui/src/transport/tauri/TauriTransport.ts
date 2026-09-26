/**
 * TauriTransport — desktop host transport (Tauri v2 IPC to `ether-native`).
 *
 * OWNERSHIP: the `native-host` node owns this folder (`ui/src/transport/tauri/**`).
 * Stub: `connect`/`send` reject with "not implemented" until that node implements it
 * (subscriptions are inert no-ops so the UI can render its connection-error state). Expected shape: commands via
 * `invoke`, events via `listen`, playhead/meters via a Tauri `Channel`; must honour the
 * `EngineTransport` contract (patches before replies, see `../EngineTransport.ts` and the
 * reference implementation `../mock/MockTransport.ts`).
 */

import type { Command, Event, MeterFrame, PlayheadFrame, Project, ReplyValue } from "@/generated";
import type { EngineTransport, SendOptions, Unsubscribe } from "../EngineTransport";

const NOT_IMPLEMENTED = "not implemented: native-host node";

export class TauriTransport implements EngineTransport {
  /** Flip to `true` once implemented (`createDefaultTransport` falls back to the mock until then). */
  static readonly implemented: boolean = false;

  readonly kind = "tauri" as const;

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
