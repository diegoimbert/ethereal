// Settings > Advanced: the engine server (remote engine), the engine log (desktop) and the
// debug panels.
import { isTauri, type EngineTransport } from "@/transport";
import { DebugPanelsSettings } from "@/features/debug";
import { EngineLogSettings } from "@/features/engine-health";
import { RemoteEngineSettings } from "@/features/remote/RemoteEngineSettings";
import { useAudioSettings } from "./store";

export function AdvancedSettings({ transport }: { transport: EngineTransport }) {
  void transport;
  return (
    <div className="eth-settings__stack" data-testid="settings-advanced">
      <RemoteEngineSettings />
      {isTauri() && <EngineLogSettings />}
      <DebugPanelsSettings onOpen={() => useAudioSettings.getState().close()} />
    </div>
  );
}
