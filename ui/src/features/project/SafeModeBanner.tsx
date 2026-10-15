import { useState } from "react";
import { ShieldAlert } from "lucide-react";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useT } from "@/i18n";

/**
 * base-131 safe mode ("Open without plugins"): while the open project's plugin devices are
 * held as bypassed placeholders, the top bar says so, with "Load plugins" to instantiate
 * them (`Project::LoadPlugins`). Nothing shows otherwise.
 */
export function SafeModeBanner() {
  const t = useT();
  const safe = useProjectStore((s) => s.safe);
  const held = useProjectStore((s) => s.safeMode.length);
  const { send, error, clearError } = useEngineCommands();
  const [busy, setBusy] = useState(false);
  if (!safe) return null;
  const load = async () => {
    setBusy(true);
    await send(cmd("Project", { type: "LoadPlugins" }));
    setBusy(false);
  };
  return (
    <div className="eth-safe-mode" role="status" aria-label={t("project.safe.label")} data-testid="safe-mode-banner">
      <ShieldAlert aria-hidden className="eth-safe-mode__icon" />
      <span className="eth-safe-mode__text" title={t("project.safe.hint", { count: held })}>
        {t("project.safe.text")}
      </span>
      <Button size="sm" tone="accent" disabled={busy} onClick={() => void load()}>
        {t("project.safe.load")}
      </Button>
      {error && (
        <button type="button" className="eth-project__error" role="alert" title={t("common.dismiss")} onClick={clearError}>
          {error}
        </button>
      )}
    </div>
  );
}
