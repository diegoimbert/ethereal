import type { EngineTransport } from "@/transport";
import { t } from "@/i18n";

/** Shown instead of learning on hosts without hardware MIDI input. */
export const webNotice = (): string => t("midi_learn.web_notice");

/**
 * Hosts that deliver hardware MIDI input: desktop (Tauri) and remote engines, and the mock
 * (which simulates input). The web build has no MIDI input yet (Web MIDI is not wired).
 */
export function canLearn(transport: EngineTransport): boolean {
  return transport.kind !== "wasm";
}

/** Shown with the mock engine (`just dev-ui`): it has no hardware input to learn from. */
export const mockNotice = (): string => t("midi_learn.mock_notice", { api: "MockTransport.simulateMidiInput" });

export function isSimulated(transport: EngineTransport): boolean {
  return transport.kind === "mock";
}
