import { useEffect, useState } from "react";
import type { RecoveryInfo } from "@/generated";
import { useEngineCommands, useOptionalConnection } from "@/features/transport-bar/engine";
import { useProjectScreen } from "@/features/project/screenStore";
import { Button, Dialog } from "@/kit";
import { cmd } from "@/transport";
import { formatExact, formatWhen } from "./format";

/**
 * Crash recovery at startup: once connected, asks the engine which projects were left
 * open by a session that didn't close cleanly and hold unsaved work newer than their saved
 * file (`Version::ListRecoverable`). For each one: "Recover" opens that work (unsaved, so
 * Save keeps it), "Keep saved" drops the offer (the versions stay in the versions dialog).
 * Closing the dialog decides nothing: it asks again next time.
 *
 * Mounted once by the app shell; it shows nothing when there is nothing to recover.
 */
export function RecoveryDialog() {
  const { transport, send, error, clearError } = useEngineCommands();
  const connected = useOptionalConnection()?.status === "connected";
  const [items, setItems] = useState<RecoveryInfo[]>([]);
  const [dismissed, setDismissed] = useState(false);
  const [busy, setBusy] = useState(false);

  // Ask once per engine connection.
  useEffect(() => {
    if (!transport || !connected) return;
    let active = true;
    void send(cmd("Version", { type: "ListRecoverable" })).then((reply) => {
      if (!active || reply?.type !== "Recoverable") return;
      setItems(reply.projects);
      setDismissed(false);
      // The recovery choice comes first; the project screen would cover it.
      if (reply.projects.length > 0) useProjectScreen.setState({ open: false, launchPending: false });
    });
    return () => {
      active = false;
    };
  }, [transport, connected, send]);

  const decide = async (item: RecoveryInfo, recover: boolean) => {
    setBusy(true);
    const reply = await send(
      recover ? cmd("Version", { type: "Recover", project: item.project }) : cmd("Version", { type: "DiscardRecovery", project: item.project }),
    );
    setBusy(false);
    if (!reply) return;
    // One project can be open: recovering one closes the dialog.
    setItems((list) => (recover ? [] : list.filter((p) => p.project !== item.project)));
  };

  const open = items.length > 0 && !dismissed;
  return (
    <Dialog
      open={open}
      onClose={() => setDismissed(true)}
      title="Recover unsaved work?"
      className="eth-recovery"
      footer={
        <Button tone="ghost" onClick={() => setDismissed(true)}>
          Decide later
        </Button>
      }
    >
      <p className="eth-recovery__intro">
        Ethereal didn’t close properly last time. {items.length === 1 ? "This project has" : "These projects have"} changes newer than the last save.
      </p>
      {error && (
        <button type="button" className="eth-versions__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}
      <ul className="eth-recovery__list" aria-label="Recoverable projects">
        {items.map((item) => (
          <li key={item.project} className="eth-recovery__item">
            <span className="eth-recovery__info">
              <span className="eth-recovery__name" title={item.name}>
                {item.name}
              </span>
              <span className="eth-recovery__meta">
                <span title={formatExact(item.version.created_ms)}>Unsaved work from {formatWhen(item.version.created_ms)}</span>
                {" · "}
                <span title={formatExact(item.saved_ms)}>last saved {formatWhen(item.saved_ms)}</span>
              </span>
            </span>
            <span className="eth-recovery__actions">
              <Button disabled={busy} aria-label={`Keep the saved ${item.name}`} onClick={() => void decide(item, false)}>
                Keep saved
              </Button>
              <Button tone="accent" disabled={busy} aria-label={`Recover ${item.name}`} onClick={() => void decide(item, true)}>
                Recover
              </Button>
            </span>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}
