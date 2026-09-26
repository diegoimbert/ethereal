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
 * - inside Tauri → `TauriTransport` (falls back to `MockTransport` with a warning until the
 *   native-host node implements it, so the desktop app still boots);
 * - `VITE_ETHER_TRANSPORT=wasm` → `WasmTransport`;
 * - otherwise → `MockTransport` (standalone UI dev, tests, demos).
 */
export function createDefaultTransport(mockOptions?: MockTransportOptions): EngineTransport {
  if (isTauri()) {
    if (TauriTransport.implemented) return new TauriTransport();
    console.warn("Ethereal: TauriTransport is not implemented yet; using MockTransport.");
    return new MockTransport(mockOptions);
  }
  if (import.meta.env.VITE_ETHER_TRANSPORT === "wasm") return new WasmTransport();
  return new MockTransport(mockOptions);
}
