import { create } from "zustand";
import type { AudioConfig } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

/**
 * Audio settings dialog state, plus the engine's last known audio config (so other parts
 * of the UI can tell, e.g., that no input device is open when a track gets armed).
 */
export interface AudioSettingsState {
  open: boolean;
  /** Why the dialog was opened ("input": a track was armed with no input device). */
  reason: "input" | null;
  /** Last config the engine reported (`Engine::ListAudioDevices`), if known. */
  config: AudioConfig | null;
  openSettings(reason?: "input"): void;
  close(): void;
  setConfig(config: AudioConfig | null): void;
}

export const useAudioSettings = create<AudioSettingsState>()((set) => ({
  open: false,
  reason: null,
  config: null,
  openSettings: (reason) => set({ open: true, reason: reason ?? null }),
  close: () => set({ open: false, reason: null }),
  setConfig: (config) => set({ config }),
}));

/** Open the audio settings dialog (e.g. from the command palette). */
export function openAudioSettings(reason?: "input"): void {
  useAudioSettings.getState().openSettings(reason);
}

/**
 * After arming an audio track: if the engine opens no input device, recordings would be
 * silent, so open the settings on the input choice. Asks the engine each time (the device
 * may have been picked elsewhere); stays quiet where devices can't be listed (browser).
 */
export async function promptForInputIfNone(transport: EngineTransport): Promise<void> {
  try {
    const reply = await transport.send(cmd("Engine", { type: "ListAudioDevices" }));
    if (reply.type !== "AudioDevices") return;
    useAudioSettings.getState().setConfig(reply.devices.current);
    if (!reply.devices.current.input_device) openAudioSettings("input");
  } catch {
    /* no device control here */
  }
}
