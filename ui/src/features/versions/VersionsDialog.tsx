import { useEffect, useState, type FormEvent, type MouseEvent } from "react";
import { MoreHorizontal } from "lucide-react";
import type { VersionDiff, VersionInfo } from "@/generated";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { Badge, Button, Dialog, DismissibleError, IconButton, openContextMenu, TextInput } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { countsLabel, formatExact, formatSize, formatWhen, isEmptyDiff, kindLabel, sortTables, tableLabel, versionTitle } from "./format";
import { useVersionsDialog } from "./store";
import { formatList, Trans, useT } from "@/i18n";

/**
 * The open project's versions (`Command::Version`): save a named version, and for each
 * stored one (newest first) compare it with the current state, restore it, rename or delete
 * it. Restoring first keeps the current state as a "Before restore" version, so a restore is
 * itself undone by restoring that one. Autosave versions are kept every minute while the
 * project changes, thinned to 30 spread over its history (CONTRACTS.md §13.11); saved
 * (named) versions stay until deleted.
 */
export function VersionsDialog() {
  const open = useVersionsDialog((s) => s.open);
  const hide = useVersionsDialog((s) => s.hide);
  const name = useProjectStore((s) => s.project?.settings.name ?? null);
  return (
    <Dialog
      open={open && name !== null}
      onClose={hide}
      title={<Trans k="versions.title" values={{ name: <span className="eth-user-text">{name ?? ""}</span> }} />}
      className="eth-versions"
    >
      {open && name !== null && <VersionsBody />}
    </Dialog>
  );
}

function VersionsBody() {
  const t = useT();
  const { send, error, clearError } = useEngineCommands();
  const projectId = useProjectStore((s) => s.project?.id ?? null);
  const [versions, setVersions] = useState<VersionInfo[] | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [comparing, setComparing] = useState<{ id: string; diff: VersionDiff | null } | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  // Listed when shown, and again whenever the engine says the versions changed.
  const [changes, setChanges] = useState(0);
  useEngineEvent((e) => {
    if (e.type === "Version") setChanges((n) => n + 1);
  });
  useEffect(() => {
    let active = true;
    void send(cmd("Version", { type: "List" })).then((reply) => {
      if (active && reply?.type === "Versions") setVersions(reply.versions);
    });
    return () => {
      active = false;
    };
  }, [send, projectId, changes]);

  const run = async (command: Parameters<typeof send>[0]) => {
    setBusy(true);
    const reply = await send(command);
    setBusy(false);
    return reply;
  };

  const save = async (e: FormEvent) => {
    e.preventDefault();
    const reply = await run(cmd("Version", { type: "Create", name: draft.trim() || null }));
    if (reply?.type === "Version") {
      setDraft("");
      setNotice(t("versions.saved", { name: versionTitle(reply.version) }));
    }
  };

  const restore = async (v: VersionInfo) => {
    const reply = await run(cmd("Version", { type: "Restore", version: v.id }));
    if (reply) {
      setComparing(null);
      setNotice(t("versions.restored", { name: versionTitle(v), when: formatWhen(v.created_ms) }));
    }
  };

  const compare = async (v: VersionInfo) => {
    if (comparing?.id === v.id) return setComparing(null);
    setComparing({ id: v.id, diff: null });
    const reply = await send(cmd("Version", { type: "Compare", version: v.id, against: null }));
    setComparing((c) => (c?.id === v.id ? { id: v.id, diff: reply?.type === "VersionDiff" ? reply.diff : null } : c));
  };

  const commitRename = () => {
    if (!renaming) return;
    const v = versions?.find((x) => x.id === renaming.id);
    setRenaming(null);
    const next = renaming.name.trim() || null;
    if (v && next !== v.name) void run(cmd("Version", { type: "Rename", version: v.id, name: next }));
  };

  const menu = (e: MouseEvent, v: VersionInfo) =>
    openContextMenu(e, [
      { label: t("versions.restore"), onSelect: () => void restore(v) },
      { label: t("versions.menu.compare"), onSelect: () => void compare(v) },
      { label: t("project.actions.rename"), onSelect: () => setRenaming({ id: v.id, name: v.name ?? "" }) },
      "separator",
      { label: t("versions.menu.delete"), danger: true, onSelect: () => setConfirmDelete(v.id) },
    ]);

  return (
    <div className="eth-versions__body">
      <form className="eth-versions__save" onSubmit={save}>
        <TextInput
          aria-label={t("versions.save.input")}
          placeholder={t("versions.save.placeholder")}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <Button type="submit" tone="accent" disabled={busy}>
          {t("versions.save.submit")}
        </Button>
      </form>

      {notice && (
        <p className="eth-versions__notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <DismissibleError className="eth-versions__error" onDismiss={clearError}>
          {error}
        </DismissibleError>
      )}

      {versions !== null && versions.length === 0 && <p className="eth-versions__empty">{t("versions.empty")}</p>}

      {versions !== null && versions.length > 0 && (
        <ul className="eth-versions__list" aria-label={t("versions.list")}>
          {versions.map((v) => (
            <li key={v.id} className="eth-versions__item" onContextMenu={(e) => menu(e, v)}>
              <div className="eth-versions__row">
                {renaming?.id === v.id ? (
                  <form
                    className="eth-versions__rename"
                    onSubmit={(e) => {
                      e.preventDefault();
                      commitRename();
                    }}
                  >
                    <TextInput
                      aria-label={t("project.actions.rename_input", { name: versionTitle(v) })}
                      autoFocus
                      placeholder={kindLabel(v.kind)}
                      value={renaming.name}
                      onChange={(e) => setRenaming({ id: v.id, name: e.target.value })}
                      onBlur={commitRename}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") {
                          e.stopPropagation(); // don't close the dialog
                          setRenaming(null);
                        }
                      }}
                    />
                  </form>
                ) : (
                  <span className="eth-versions__info">
                    <span className={v.name !== null ? "eth-versions__title eth-user-text" : "eth-versions__title"} title={versionTitle(v)}>
                      {versionTitle(v)}
                    </span>
                    <span className="eth-versions__meta" title={`${formatExact(v.created_ms)} · ${formatSize(v.size)}`}>
                      {formatWhen(v.created_ms)}
                      {v.name !== null && v.kind !== "Manual" && <> · {kindLabel(v.kind)}</>}
                    </span>
                  </span>
                )}
                {v.kind === "BeforeRestore" && <Badge tone="warn">{kindLabel("BeforeRestore")}</Badge>}
                {confirmDelete === v.id ? (
                  <span className="eth-versions__actions">
                    <Button size="sm" onClick={() => setConfirmDelete(null)}>
                      {t("project.actions.cancel")}
                    </Button>
                    <Button
                      size="sm"
                      tone="danger"
                      disabled={busy}
                      aria-label={t("project.actions.confirm_delete", { name: versionTitle(v) })}
                      onClick={() => {
                        setConfirmDelete(null);
                        if (comparing?.id === v.id) setComparing(null);
                        void run(cmd("Version", { type: "Delete", version: v.id }));
                      }}
                    >
                      {t("project.actions.delete")}
                    </Button>
                  </span>
                ) : (
                  <span className="eth-versions__actions">
                    <Button
                      size="sm"
                      tone="ghost"
                      active={comparing?.id === v.id}
                      aria-label={t("versions.compare_named", { name: versionTitle(v) })}
                      onClick={() => void compare(v)}
                    >
                      {t("versions.compare")}
                    </Button>
                    <Button size="sm" disabled={busy} aria-label={t("versions.restore_named", { name: versionTitle(v) })} onClick={() => void restore(v)}>
                      {t("versions.restore")}
                    </Button>
                    <IconButton
                      size="sm"
                      tone="ghost"
                      label={t("project.actions.more", { name: versionTitle(v) })}
                      icon={<MoreHorizontal aria-hidden />}
                      onClick={(e) => {
                        const r = e.currentTarget.getBoundingClientRect();
                        menu({ clientX: r.left, clientY: r.bottom, preventDefault: () => undefined, stopPropagation: () => undefined } as MouseEvent, v);
                      }}
                    />
                  </span>
                )}
              </div>
              {comparing?.id === v.id && <DiffView diff={comparing.diff} />}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** What changed from the version to now. */
function DiffView({ diff }: { diff: VersionDiff | null }) {
  const t = useT();
  if (diff === null) return <p className="eth-versions__diff eth-versions__diff--empty">{t("versions.diff.comparing")}</p>;
  if (isEmptyDiff(diff)) return <p className="eth-versions__diff eth-versions__diff--empty">{t("versions.diff.same")}</p>;
  return (
    <div className="eth-versions__diff" aria-label={t("versions.diff.label")}>
      <span className="eth-versions__diff-caption">{t("versions.diff.caption")}</span>
      <ul className="eth-versions__diff-list">
        {sortTables(diff).map((table) => (
          <li key={table.table} className="eth-versions__diff-row">
            <span className="eth-versions__diff-table">{tableLabel(table.table)}</span>
            <span className="eth-versions__diff-counts" title={t("versions.diff.counts")}>
              {countsLabel(table)}
            </span>
            {table.names.length > 0 && <span className="eth-versions__diff-names eth-user-text">{formatList(table.names)}</span>}
          </li>
        ))}
        {diff.settings_changed && (
          <li className="eth-versions__diff-row">
            <span className="eth-versions__diff-table">{t("versions.diff.settings")}</span>
            <span className="eth-versions__diff-counts">{t("versions.diff.changed")}</span>
          </li>
        )}
      </ul>
    </div>
  );
}
