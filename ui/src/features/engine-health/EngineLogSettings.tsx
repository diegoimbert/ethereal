// Settings > Advanced > Engine log (desktop): opens a terminal that follows the engine's log
// file (release builds have no console). The log has the engine's warnings, including the
// watchdog's "engine unresponsive" lines with their reason.
import { useState } from "react";
import { useT } from "@/i18n";
import { Button } from "@/kit";
import { openEngineLog } from "./engineLog";

export function EngineLogSettings() {
  const t = useT();
  const [error, setError] = useState<string | null>(null);
  const open = () => {
    setError(null);
    openEngineLog().catch((e: unknown) => setError(String(e)));
  };
  return (
    <section className="eth-settings__section" aria-label={t("shell.engine_log.heading")} data-testid="settings-engine-log">
      <h3 className="eth-settings__heading">{t("shell.engine_log.heading")}</h3>
      <p className="eth-settings__hint">{t("shell.engine_log.hint")}</p>
      <div>
        <Button size="sm" onClick={open}>
          {t("shell.engine_log.open")}
        </Button>
      </div>
      {error && (
        <p className="eth-settings__hint" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
