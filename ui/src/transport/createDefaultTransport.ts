import type { EngineTransport } from "./EngineTransport";
import { MockTransport, type MockTransportOptions } from "./mock/MockTransport";
import { TauriTransport } from "./tauri/TauriTransport";
import { WasmTransport } from "./wasm/WasmTransport";

/** `true` when running inside the Tauri desktop shell. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Pick the transport for the current environment:
 * - inside Tauri → `TauriTransport` (native host → real controller → engine);
 * - `VITE_ETHER_TRANSPORT=wasm` → `WasmTransport`;
 * - otherwise → `MockTransport` (standalone UI dev via `just dev-ui`, tests, demos).
 */
export function createDefaultTransport(mockOptions?: MockTransportOptions): EngineTransport {
  if (isTauri()) return new TauriTransport();
  if (import.meta.env.VITE_ETHER_TRANSPORT === "wasm") return new WasmTransport();
  return new MockTransport(mockOptions);
}
