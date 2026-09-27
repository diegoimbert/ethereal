import type { EngineTransport } from "@/transport";

/** Shown instead of learning on hosts without hardware MIDI input. */
export const WEB_NOTICE = "MIDI learn needs the desktop app";

/**
 * Hosts that deliver hardware MIDI input: desktop (Tauri) and remote engines, and the mock
 * (which simulates input). The web build has no MIDI input yet (Web MIDI is not wired).
 */
export function canLearn(transport: EngineTransport): boolean {
  return transport.kind !== "wasm";
}

/** Shown with the mock engine (`just dev-ui`): it has no hardware input to learn from. */
export const MOCK_NOTICE = "Demo engine: no hardware MIDI input here. Learning completes only with simulated input (MockTransport.simulateMidiInput).";

export function isSimulated(transport: EngineTransport): boolean {
  return transport.kind === "mock";
}
