import { useEffect } from "react";
import type { EngineTransport } from "@/transport";
import { notify, useNotices } from "@/features/notifications/store";
import { t } from "@/i18n";
import { EngineHealthWatcher } from "./watcher";

export { EngineLogSettings } from "./EngineLogSettings";
export { engineLogIpc, openEngineLog } from "./engineLog";
export { EngineHealthWatcher, SETTLE_MS, SILENCE_MS } from "./watcher";
export type { HealthUi } from "./watcher";

const CHECK_MS = 500;

/**
 * Watch `transport` for engine stalls and show them as notices (see `watcher.ts`).
 * Returns the cleanup.
 *
 * Meter silence only counts on desktop: there the engine streams meters steadily (and its
 * events can't get through a blocked main thread), while other transports may pause the
 * meters when everything is silent; their engine events still arrive.
 */
export function watchEngineHealth(transport: EngineTransport): () => void {
  const silenceMeansStuck = transport.kind === "tauri";
  const watcher = new EngineHealthWatcher({
    warn: (message) => notify("Warning", message, { title: t("shell.engine_health.stalled_title") }),
    dismiss: (id) => useNotices.getState().dismiss(id),
    info: (message) => notify("Info", message, { title: t("shell.engine_health.recovered_title") }),
  });
  const offMeters = silenceMeansStuck ? transport.subscribeMeters(() => watcher.frame()) : () => {};
  const offEvent = transport.onEvent((e) => {
    if (e.type === "Engine") watcher.event(e.event);
  });
  const onVisibility = () => {
    if (document.visibilityState === "hidden") watcher.hidden();
  };
  document.addEventListener("visibilitychange", onVisibility);
  const timer = setInterval(() => watcher.check(), CHECK_MS);
  return () => {
    clearInterval(timer);
    document.removeEventListener("visibilitychange", onVisibility);
    offEvent();
    offMeters();
  };
}

/** `watchEngineHealth` while mounted. */
export function useEngineHealth(transport: EngineTransport | null): void {
  useEffect(() => (transport ? watchEngineHealth(transport) : undefined), [transport]);
}
