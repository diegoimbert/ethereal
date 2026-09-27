import { create } from "zustand";
import type { AudioConfig, AudioDeviceList, EngineStatus } from "@/generated";
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
  /** Devices and status, loaded when the app connects (so the dialog opens ready) and on refresh. */
  devices: AudioDeviceList | null;
  status: EngineStatus | null;
  /** Why devices can't be listed here (e.g. the browser), if so. */
  unavailable: string | null;
  loading: boolean;
  openSettings(reason?: "input"): void;
  close(): void;
  setConfig(config: AudioConfig | null): void;
}

export const useAudioSettings = create<AudioSettingsState>()((set) => ({
  open: false,
  reason: null,
  config: null,
  devices: null,
  status: null,
  unavailable: null,
  loading: false,
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

/**
 * Load (or refresh) the device list and the engine status into the store. Called once when
 * the app connects and by the dialog's refresh button / after each change.
 */
export async function loadAudioDevices(transport: EngineTransport): Promise<void> {
  useAudioSettings.setState({ loading: true });
  const [devices, status] = await Promise.allSettled([
    transport.send(cmd("Engine", { type: "ListAudioDevices" })),
    transport.send(cmd("Engine", { type: "GetStatus" })),
  ]);
  const patch: Partial<AudioSettingsState> = { loading: false };
  if (devices.status === "fulfilled" && devices.value.type === "AudioDevices") {
    patch.devices = devices.value.devices;
    patch.config = devices.value.devices.current;
    patch.unavailable = null;
  } else if (devices.status === "rejected") {
    const e = devices.reason as { error?: { message?: string; code?: string } } | Error;
    patch.unavailable = "error" in e && e.error ? e.error.message || e.error.code || "unavailable" : String((e as Error).message ?? e);
  }
  if (status.status === "fulfilled" && status.value.type === "Status") patch.status = status.value.status;
  useAudioSettings.setState(patch);
}
