import { useEffect, useState } from "react";
import type { Command, ProjectId, RecoveryInfo } from "@/generated";
import { useProjectScreen } from "@/features/project/screenStore";
import { useEngineCommands, useOptionalConnection } from "@/features/transport-bar/engine";
import { Button, Dialog, DismissibleError } from "@/kit";
import { cmd, listProjects } from "@/transport";
import { formatExact, formatWhen } from "./format";
import { previousSession } from "./session";
import { useRecoveryShown } from "./store";
import { useT } from "@/i18n";

/** A project the last session left open: with unsaved work to recover, or without. */
interface Item {
  project: ProjectId;
  name: string;
  /** Unsaved work newer than the saved file (`ListRecoverable`), if any. */
  recovery: RecoveryInfo | null;
  saved_ms: number;
}

/**
 * Crash recovery at startup: once connected, asks which projects were left open by a
 * session that didn't close cleanly (`previousSession`) and which hold
 * unsaved work newer than their saved file (`Version::ListRecoverable`). For each one:
 * - with unsaved work: "Recover" opens that work (unsaved, so Save keeps it), "Keep saved"
 *   drops the offer (the versions stay in the versions dialog);
 * - without (e.g. a plugin crashed while it loaded): "Open", or "Dismiss";
 * - both: "Open without plugins" opens the saved file in safe mode (plugin devices held as
 *   bypassed placeholders until "Load plugins").
 * Closing the dialog decides nothing: it asks again next time. The project screen waits
 * behind it (`useRecoveryShown`).
 *
 * Mounted once by the app shell; it shows nothing when there is nothing to recover.
 */
export function RecoveryDialog() {
  const t = useT();
  const { transport, send, error, clearError } = useEngineCommands();
  const connected = useOptionalConnection()?.status === "connected";
  const [items, setItems] = useState<Item[]>([]);
  const [dismissed, setDismissed] = useState(false);
  const [busy, setBusy] = useState(false);

  // Ask once per engine connection.
  useEffect(() => {
    if (!transport || !connected) return;
    let active = true;
    void Promise.all([
      send(cmd("Version", { type: "ListRecoverable" })),
      previousSession(transport).catch(() => null),
      listProjects(transport).catch(() => null),
    ]).then(([recoverable, session, list]) => {
      if (!active || recoverable?.type !== "Recoverable") return;
      const projects = list?.type === "Projects" ? list.projects : [];
      const found: Item[] = recoverable.projects.map((r) => ({ project: r.project, name: r.name, recovery: r, saved_ms: r.saved_ms }));
      for (const id of session?.unclean ?? []) {
        const p = projects.find((p) => p.id === id);
        if (p && !found.some((i) => i.project === id)) found.push({ project: id, name: p.name, recovery: null, saved_ms: p.modified_ms });
      }
      setItems(found);
      setDismissed(false);
    });
    return () => {
      active = false;
    };
  }, [transport, connected, send]);

  const open = items.length > 0 && !dismissed;
  useEffect(() => {
    useRecoveryShown.setState({ shown: open });
    return () => useRecoveryShown.setState({ shown: false });
  }, [open]);

  const run = async (command: Command, opens: boolean, item: Item) => {
    setBusy(true);
    const reply = await send(command);
    setBusy(false);
    if (!reply) return;
    // One project can be open: opening one closes the dialog (and the launch project
    // screen waiting behind it).
    if (opens) useProjectScreen.getState().hide();
    setItems((list) => (opens ? [] : list.filter((p) => p.project !== item.project)));
  };

  const anyWork = items.some((i) => i.recovery);
  return (
    <Dialog
      open={open}
      onClose={() => setDismissed(true)}
      title={anyWork ? t("versions.recovery.title_work") : t("versions.recovery.title_crash")}
      className="eth-recovery"
      footer={
        <Button tone="ghost" onClick={() => setDismissed(true)}>
          {t("versions.recovery.later")}
        </Button>
      }
    >
      <p className="eth-recovery__intro">
        {anyWork ? t("versions.recovery.intro_work", { count: items.length }) : t("versions.recovery.intro_crash", { count: items.length })}
      </p>
      {error && (
        <DismissibleError className="eth-versions__error" onDismiss={clearError}>
          {error}
        </DismissibleError>
      )}
      <ul className="eth-recovery__list" aria-label={t("versions.recovery.list")}>
        {items.map((item) => (
          <li key={item.project} className="eth-recovery__item">
            <span className="eth-recovery__info">
              <span className="eth-recovery__name eth-user-text" title={item.name}>
                {item.name}
              </span>
              <span className="eth-recovery__meta">
                {item.recovery ? (
                  <span title={formatExact(item.recovery.version.created_ms)}>
                    {t("versions.recovery.unsaved_from", { when: formatWhen(item.recovery.version.created_ms) })}
                  </span>
                ) : (
                  <span>{t("versions.recovery.no_unsaved")}</span>
                )}
                {" · "}
                <span title={formatExact(item.saved_ms)}>{t("versions.recovery.last_saved", { when: formatWhen(item.saved_ms) })}</span>
              </span>
            </span>
            <span className="eth-recovery__actions">
              {item.recovery ? (
                <Button
                  disabled={busy}
                  aria-label={t("versions.recovery.keep_saved_named", { name: item.name })}
                  onClick={() => void run(cmd("Version", { type: "DiscardRecovery", project: item.project }), false, item)}
                >
                  {t("versions.recovery.keep_saved")}
                </Button>
              ) : (
                <Button
                  tone="ghost"
                  disabled={busy}
                  aria-label={t("versions.recovery.dismiss_named", { name: item.name })}
                  onClick={() => void run(cmd("Version", { type: "DiscardRecovery", project: item.project }), false, item)}
                >
                  {t("common.dismiss")}
                </Button>
              )}
              <Button
                disabled={busy}
                aria-label={t("versions.recovery.open_safe_named", { name: item.name })}
                title={t("versions.recovery.open_safe_hint")}
                onClick={() => void run(cmd("Project", { type: "OpenMirrored", id: item.project, safe: true }), true, item)}
              >
                {t("project.actions.open_safe")}
              </Button>
              {item.recovery ? (
                <Button
                  tone="accent"
                  disabled={busy}
                  aria-label={t("versions.recovery.recover_named", { name: item.name })}
                  onClick={() => void run(cmd("Version", { type: "Recover", project: item.project }), true, item)}
                >
                  {t("versions.recovery.recover")}
                </Button>
              ) : (
                <Button
                  tone="accent"
                  disabled={busy}
                  aria-label={t("project.actions.open_named", { name: item.name })}
                  onClick={() => void run(cmd("Project", { type: "OpenMirrored", id: item.project }), true, item)}
                >
                  {t("project.actions.open")}
                </Button>
              )}
            </span>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}
