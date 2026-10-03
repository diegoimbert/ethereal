import { useEffect, useState, type FormEvent, type MouseEvent } from "react";
import { MoreHorizontal } from "lucide-react";
import type { VersionDiff, VersionInfo } from "@/generated";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { Badge, Button, Dialog, IconButton, openContextMenu, TextInput } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { countsLabel, formatExact, formatSize, formatWhen, isEmptyDiff, kindLabel, sortTables, tableLabel, versionTitle } from "./format";
import { useVersionsDialog } from "./store";

/**
 * The open project's versions (`Command::Version`): save a named version, and for each
 * stored one (newest first) compare it with the current state, restore it, rename or delete
 * it. Restoring first keeps the current state as a "Before restore" version, so a restore is
 * itself undone by restoring that one. Autosave versions roll every 5 minutes while the
 * project changes (the newest 50 are kept); saved versions stay until deleted.
 */
export function VersionsDialog() {
  const open = useVersionsDialog((s) => s.open);
  const hide = useVersionsDialog((s) => s.hide);
  const name = useProjectStore((s) => s.project?.settings.name ?? null);
  return (
    <Dialog open={open && name !== null} onClose={hide} title={`Versions of “${name ?? ""}”`} className="eth-versions">
      {open && name !== null && <VersionsBody />}
    </Dialog>
  );
}

function VersionsBody() {
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
      setNotice(`Saved “${versionTitle(reply.version)}”.`);
    }
  };

  const restore = async (v: VersionInfo) => {
    const reply = await run(cmd("Version", { type: "Restore", version: v.id }));
    if (reply) {
      setComparing(null);
      setNotice(`Restored “${versionTitle(v)}” (${formatWhen(v.created_ms)}). The state before it is kept as a “Before restore” version.`);
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
      { label: "Restore", onSelect: () => void restore(v) },
      { label: "Compare with current", onSelect: () => void compare(v) },
      { label: "Rename", onSelect: () => setRenaming({ id: v.id, name: v.name ?? "" }) },
      "separator",
      { label: "Delete…", danger: true, onSelect: () => setConfirmDelete(v.id) },
    ]);

  return (
    <div className="eth-versions__body">
      <form className="eth-versions__save" onSubmit={save}>
        <TextInput
          aria-label="Version name"
          placeholder="Name this version (optional)"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <Button type="submit" tone="accent" disabled={busy}>
          Save version
        </Button>
      </form>

      {notice && (
        <p className="eth-versions__notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <button type="button" className="eth-versions__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}

      {versions !== null && versions.length === 0 && (
        <p className="eth-versions__empty">No versions yet. While you work, a version is kept every 5 minutes; save one now to mark a moment you may want back.</p>
      )}

      {versions !== null && versions.length > 0 && (
        <ul className="eth-versions__list" aria-label="Versions">
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
                      aria-label={`New name for ${versionTitle(v)}`}
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
                    <span className="eth-versions__title" title={versionTitle(v)}>
                      {versionTitle(v)}
                    </span>
                    <span className="eth-versions__meta" title={`${formatExact(v.created_ms)} · ${formatSize(v.size)}`}>
                      {formatWhen(v.created_ms)}
                      {v.name !== null && v.kind !== "Manual" && <> · {kindLabel(v.kind)}</>}
                    </span>
                  </span>
                )}
                {v.kind === "BeforeRestore" && <Badge tone="warn">Before restore</Badge>}
                {confirmDelete === v.id ? (
                  <span className="eth-versions__actions">
                    <Button size="sm" onClick={() => setConfirmDelete(null)}>
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      tone="danger"
                      disabled={busy}
                      aria-label={`Confirm delete ${versionTitle(v)}`}
                      onClick={() => {
                        setConfirmDelete(null);
                        if (comparing?.id === v.id) setComparing(null);
                        void run(cmd("Version", { type: "Delete", version: v.id }));
                      }}
                    >
                      Delete
                    </Button>
                  </span>
                ) : (
                  <span className="eth-versions__actions">
                    <Button size="sm" tone="ghost" active={comparing?.id === v.id} aria-label={`Compare ${versionTitle(v)} with now`} onClick={() => void compare(v)}>
                      Compare
                    </Button>
                    <Button size="sm" disabled={busy} aria-label={`Restore ${versionTitle(v)}`} onClick={() => void restore(v)}>
                      Restore
                    </Button>
                    <IconButton
                      size="sm"
                      tone="ghost"
                      label={`More actions for ${versionTitle(v)}`}
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
  if (diff === null) return <p className="eth-versions__diff eth-versions__diff--empty">Comparing…</p>;
  if (isEmptyDiff(diff)) return <p className="eth-versions__diff eth-versions__diff--empty">Same as now.</p>;
  return (
    <div className="eth-versions__diff" aria-label="Changes since this version">
      <span className="eth-versions__diff-caption">Since this version</span>
      <ul className="eth-versions__diff-list">
        {sortTables(diff).map((t) => (
          <li key={t.table} className="eth-versions__diff-row">
            <span className="eth-versions__diff-table">{tableLabel(t.table)}</span>
            <span className="eth-versions__diff-counts" title="added, removed, changed">
              {countsLabel(t)}
            </span>
            {t.names.length > 0 && <span className="eth-versions__diff-names">{t.names.join(", ")}</span>}
          </li>
        ))}
        {diff.settings_changed && (
          <li className="eth-versions__diff-row">
            <span className="eth-versions__diff-table">Project settings</span>
            <span className="eth-versions__diff-counts">changed</span>
          </li>
        )}
      </ul>
    </div>
  );
}
