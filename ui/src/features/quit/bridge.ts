// Desktop quit confirmation: the shell's side (`apps/desktop/src-tauri/src/quit.rs`).
// ⌘Q and the main window's close button emit `QUIT_REQUESTED_EVENT` (once the UI said it
// listens, `app_quit_guard`); the UI answers with `app_quit` / `app_quit_cancel`. ⌘W in the
// main window emits `CLOSE_REQUESTED_EVENT`. The web build has no bridge (`quitBridge()` is
// null) and nothing changes there. Tests swap in their own bridge (`setQuitBridge`).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "@/transport";

export const QUIT_REQUESTED_EVENT = "ether://quit-requested";
export const CLOSE_REQUESTED_EVENT = "ether://close-requested";

export interface QuitBridge {
  /** Calls `handler` on each quit request; resolves with the unsubscribe. */
  onQuitRequested(handler: () => void): Promise<() => void>;
  /** Calls `handler` on each ⌘W in the main window; resolves with the unsubscribe. */
  onCloseRequested(handler: () => void): Promise<() => void>;
  /** The UI listens (`true`) or stopped listening: the shell asks only while it does. */
  setGuard(ready: boolean): Promise<void>;
  /** Quit now (the engine stops and saves on the way out). */
  quit(): Promise<void>;
  /** Keep running. */
  cancel(): Promise<void>;
}

export const tauriQuitBridge: QuitBridge = {
  onQuitRequested: (handler) => listen(QUIT_REQUESTED_EVENT, () => handler()),
  onCloseRequested: (handler) => listen(CLOSE_REQUESTED_EVENT, () => handler()),
  setGuard: (ready) => invoke<void>("app_quit_guard", { ready }),
  quit: () => invoke<void>("app_quit"),
  cancel: () => invoke<void>("app_quit_cancel"),
};

let override: QuitBridge | null | undefined;

/** Tests: use `bridge` (null: as in the web build); `undefined` restores the default. */
export function setQuitBridge(bridge: QuitBridge | null | undefined): void {
  override = bridge;
}

/** The bridge, or null outside the desktop shell. */
export function quitBridge(): QuitBridge | null {
  if (override !== undefined) return override;
  return isTauri() ? tauriQuitBridge : null;
}
