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
