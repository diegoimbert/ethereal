import { useState } from "react";
import { ShieldAlert } from "lucide-react";
import { notify } from "@/features/notifications";
import { StatusButton, StatusText } from "@/features/status-bar/StatusButton";
import { errorMessage, useOptionalTransport } from "@/features/transport-bar/engine";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useT } from "@/i18n";

/**
 * Safe mode ("Open without plugins"): while the open project's plugin devices are held as
 * bypassed placeholders, the status bar says so, with "load plugins" to instantiate them
 * (`Project::LoadPlugins`; a failure is a toast). Nothing shows otherwise.
 */
export function SafeModeStatus() {
  const t = useT();
  const safe = useProjectStore((s) => s.safe);
  const held = useProjectStore((s) => s.safeMode.length);
  const transport = useOptionalTransport();
  const [busy, setBusy] = useState(false);
  if (!safe) return null;
  const load = async () => {
    if (!transport) return;
    setBusy(true);
    try {
      await transport.send(cmd("Project", { type: "LoadPlugins" }));
    } catch (e) {
      notify("Error", errorMessage(e));
    }
    setBusy(false);
  };
  return (
    <span className="eth-status-bar__group" role="group" aria-label={t("project.safe.label")} data-testid="safe-mode-banner">
      <StatusText tone="warn" icon={<ShieldAlert aria-hidden />} title={t("project.safe.hint", { count: held })}>
        {t("project.safe.text")}
      </StatusText>
      <StatusButton className="eth-status-bar__action" disabled={busy} onClick={() => void load()}>
        {t("project.safe.load")}
      </StatusButton>
    </span>
  );
}
