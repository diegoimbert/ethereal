import { useEffect, useState } from "react";
import { formatBytes, useT, type MessageKey } from "@/i18n";
import { Button } from "@/kit";
import { refreshStorageStatus, requestPersistence, useStorageStatus, type Persistence } from "./persist";

const STATE: Record<Persistence, MessageKey> = {
  unknown: "storage.state.unknown",
  persistent: "storage.state.persistent",
  "best-effort": "storage.state.best_effort",
  unsupported: "storage.state.unsupported",
};

/** Settings > General, in the browser (#622): whether projects are kept, the usage, "request again". */
export function StorageSettings() {
  const { persistence, usage, quota } = useStorageStatus();
  const [asking, setAsking] = useState(false);
  const t = useT();
  useEffect(() => void refreshStorageStatus(), []);
  return (
    <section className="eth-settings__section" aria-label={t("storage.title")} data-testid="settings-storage">
      <h3 className="eth-settings__heading">{t("storage.title")}</h3>
      <div className="eth-settings__row">
        <span role="status">{t("storage.status", { state: t(STATE[persistence]) })}</span>
        {persistence === "best-effort" && (
          <Button
            size="sm"
            disabled={asking}
            onClick={() => {
              setAsking(true);
              void requestPersistence().finally(() => setAsking(false));
            }}
          >
            {t("storage.request_again")}
          </Button>
        )}
      </div>
      {usage !== null && quota !== null && (
        <p className="eth-settings__hint">{t("storage.usage", { used: formatBytes(usage, { binary: true }), quota: formatBytes(quota, { binary: true }) })}</p>
      )}
      {persistence !== "persistent" && persistence !== "unknown" && <p className="eth-settings__hint">{t("storage.browser_only")}</p>}
    </section>
  );
}
