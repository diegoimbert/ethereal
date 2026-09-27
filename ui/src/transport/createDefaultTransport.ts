import { createContext, useContext } from "react";
import type { EngineTransport } from "./EngineTransport";
import { MockTransport, type MockTransportOptions } from "./mock/MockTransport";
import { TauriTransport } from "./tauri/TauriTransport";

/** `true` when running inside the Tauri desktop shell. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Transport for the `ui/` entry (`ui/src/main.tsx`):
 * - inside Tauri (`just dev-desktop`) → `TauriTransport` (native host → real controller →
 *   engine);
 * - otherwise → `MockTransport` (standalone UI dev via `just dev-ui`, tests, demos).
 *
 * The browser build (`apps/web`, `just dev-web`) builds its own `WasmTransport` over the
 * wasm engine endpoint.
 */
export function createDefaultTransport(mockOptions?: MockTransportOptions): EngineTransport {
  if (isTauri()) return new TauriTransport();
  return new MockTransport(mockOptions);
}

/**
 * Runtime transport switching (remote engine): the app starts on the `transport` prop
 * (local engine or mock); the connect dialog can swap in an opened remote transport and
 * back. The provider owns the remote transport (it disposes it when switching back or
 * unmounting); the local one stays owned by the caller and is reconnected on return.
 */
export interface TransportSwitch {
  /** The transport the app was started with. */
  local: EngineTransport;
  /** The remote transport in use, or `null`. */
  remote: EngineTransport | null;
  /** Use `remote` (already opened) instead of the local transport. */
  switchToRemote(remote: EngineTransport): void;
  /** Back to the local transport (disposes the remote one). */
  switchToLocal(): void;
}

export const TransportSwitchContext = createContext<TransportSwitch | null>(null);

/** Transport switching, or `null` outside a `TransportProvider`. */
export function useTransportSwitch(): TransportSwitch | null {
  return useContext(TransportSwitchContext);
}
