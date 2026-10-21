// Settings > Advanced: the engine server (remote engine); Diagnostics: the engine log
// (desktop) and the debug panels.
import { isTauri, type EngineTransport } from "@/transport";
import { DebugPanelsSettings } from "@/features/debug";
import { EngineLogSettings } from "@/features/engine-health";
import { RemoteEngineSettings, useEngineServerSettingsShown } from "@/features/remote/RemoteEngineSettings";
import { useT } from "@/i18n";
import { useAudioSettings } from "./store";

export function AdvancedSettings({ transport }: { transport: EngineTransport }) {
  void transport;
  const t = useT();
  const shown = useEngineServerSettingsShown();
  return (
    <div className="eth-settings__stack" data-testid="settings-advanced">
      <RemoteEngineSettings />
      {/* Hidden until turned on: say how (never an empty page). */}
      {!shown && <p className="eth-settings__hint">{t("settings.engine.hidden_hint")}</p>}
    </div>
  );
}

export function DiagnosticsSettings() {
  return (
    <div className="eth-settings__stack" data-testid="settings-diagnostics">
      {isTauri() && <EngineLogSettings />}
      <DebugPanelsSettings onOpen={() => useAudioSettings.getState().close()} />
    </div>
  );
}
