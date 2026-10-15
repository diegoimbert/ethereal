import { useEffect, useState, type FormEvent } from "react";
import { matchesAction } from "@/features/keymap";
import { notify } from "@/features/notifications";
import { errorMessage, isTextEntry, useEngineCommands } from "@/features/transport-bar/engine";
import { Button, Dialog, TextInput } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { openSnapshotPrompt, useSnapshotPrompt } from "./store";
import { t as translate, useT } from "@/i18n";

/**
 * "Save snapshot…": a named version of the open project (`Version::Create { name }`), kept
 * until deleted (autosave versions are thinned, named ones never). A toast confirms it.
 * Opened by the palette, the project menu and the keymap's `project.snapshot` (Mod+Alt+S,
 * answered here; not from text fields, where Ctrl+Alt is AltGr on some layouts).
 */
export function SnapshotPrompt() {
  const t = useT();
  const open = useSnapshotPrompt((s) => s.open);
  const hide = useSnapshotPrompt((s) => s.hide);
  const hasProject = useProjectStore((s) => s.project !== null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || isTextEntry(e.target) || !matchesAction("project.snapshot", e)) return;
      if (useProjectStore.getState().project === null) return;
      e.preventDefault();
      if (!e.repeat) openSnapshotPrompt();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <Dialog open={open && hasProject} onClose={hide} title={t("versions.snapshot.title")} className="eth-snapshot" closeButton={false}>
      {open && hasProject && <SnapshotForm onClose={hide} />}
    </Dialog>
  );
}

function SnapshotForm({ onClose }: { onClose(): void }) {
  const t = useT();
  const { transport } = useEngineCommands();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmed = name.trim();

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (!transport || busy || !trimmed) return;
    setBusy(true);
    setError(null);
    transport
      .send(cmd("Version", { type: "Create", name: trimmed }))
      .then(() => {
        onClose();
        notify("Info", translate("versions.snapshot.saved", { name: trimmed }));
      })
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  };

  return (
    <form className="eth-snapshot__form" onSubmit={submit}>
      <p className="eth-snapshot__hint">{t("versions.snapshot.hint")}</p>
      <TextInput
        autoFocus
        aria-label={t("versions.snapshot.input")}
        placeholder={t("versions.snapshot.placeholder")}
        maxLength={120}
        value={name}
        onChange={(e) => setName(e.target.value)}
      />
      {error && (
        <p className="eth-snapshot__error" role="alert">
          {error}
        </p>
      )}
      <div className="eth-snapshot__actions">
        <Button type="button" tone="ghost" onClick={onClose}>
          {t("project.actions.cancel")}
        </Button>
        <Button type="submit" tone="accent" disabled={busy || !transport || !trimmed}>
          {t("versions.snapshot.submit")}
        </Button>
      </div>
    </form>
  );
}
