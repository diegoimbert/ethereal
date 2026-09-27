import { useEffect, useMemo, useState, type FormEvent } from "react";
import { X } from "lucide-react";
import { ProjectScale } from "@/features/scale/ProjectScale";
import type { ProjectSummary } from "@/generated";
import type { EngineCommands } from "@/features/transport-bar/engine";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newProjectId } from "@/transport";
import { copyName, formatModified, sortProjects, uniqueName } from "./projectNames";

export interface ProjectManagerProps {
  commands: EngineCommands;
  onClose(): void;
}

/** Content of the Projects popover (kit Popover, see ProjectMenu): lists the engine-side project store, with create/open/save-as/duplicate/rename/delete. */
export function ProjectManager({ commands, onClose }: ProjectManagerProps) {
  const { send, error, clearError } = commands;
  const projects = useProjectStore((s) => s.projects);
  const currentId = useProjectStore((s) => s.project?.id ?? null);
  const currentName = useProjectStore((s) => s.project?.settings.name ?? "");
  const sorted = useMemo(() => sortProjects(projects), [projects]);
  const [busy, setBusy] = useState(false);
  const [newName, setNewName] = useState("");
  const [saveAsName, setSaveAsName] = useState("");
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  // Refresh the list when opened (it is also kept current by `Project::ListChanged`).
  useEffect(() => {
    let active = true;
    void send(cmd("Project", { type: "List" })).then((reply) => {
      if (active && reply?.type === "Projects") useProjectStore.getState().setProjects(reply.projects);
    });
    return () => {
      active = false;
    };
  }, [send]);

  // Escape closes (an inline rename swallows its own Escape).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  /** Run a store command; `close` closes the popover on success. */
  const run = async (command: Parameters<typeof send>[0], close = false) => {
    setBusy(true);
    const reply = await send(command);
    setBusy(false);
    if (reply && close) onClose();
    return reply;
  };

  const create = (e: FormEvent) => {
    e.preventDefault();
    const name = uniqueName(newName || "Untitled", projects);
    void run(cmd("Project", { type: "Create", id: newProjectId(), name }), true).then((r) => r && setNewName(""));
  };

  const saveAs = (e: FormEvent) => {
    e.preventDefault();
    const name = uniqueName(saveAsName || currentName, projects);
    void run(cmd("Project", { type: "SaveAs", new_id: newProjectId(), name }), true).then((r) => r && setSaveAsName(""));
  };

  const commitRename = (e: FormEvent) => {
    e.preventDefault();
    if (!renaming) return;
    const name = renaming.name.trim();
    const target = renaming.id;
    setRenaming(null);
    if (!name || name === projects.find((p) => p.id === target)?.name) return;
    void run(cmd("Project", { type: "Rename", id: target, name }));
  };

  const duplicate = (p: ProjectSummary) =>
    void run(cmd("Project", { type: "Duplicate", id: p.id, new_id: newProjectId(), name: copyName(p.name, projects) }));

  const remove = (p: ProjectSummary) => {
    if (confirmDelete !== p.id) {
      setConfirmDelete(p.id);
      return;
    }
    setConfirmDelete(null);
    void run(cmd("Project", { type: "Delete", id: p.id }));
  };

  return (
    <div className="eth-project-mgr">
      <header className="eth-project-mgr__header">
        <span>Projects</span>
        <Button size="sm" variant="ghost" aria-label="Close" onClick={onClose}>
          <X aria-hidden />
        </Button>
      </header>
      <ProjectScale send={send} />

      <form className="eth-project-mgr__row" onSubmit={create}>
        <input
          className="eth-project-mgr__input"
          aria-label="New project name"
          placeholder="New project name"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
        />
        <Button type="submit" size="sm" disabled={busy}>
          New
        </Button>
      </form>
      <form className="eth-project-mgr__row" onSubmit={saveAs}>
        <input
          className="eth-project-mgr__input"
          aria-label="Save as name"
          placeholder={currentName ? `${currentName} (save as…)` : "Save as…"}
          value={saveAsName}
          onChange={(e) => setSaveAsName(e.target.value)}
        />
        <Button type="submit" size="sm" disabled={busy || currentId === null}>
          Save as
        </Button>
      </form>

      {error && (
        <button type="button" className="eth-project__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}

      <ul className="eth-project-mgr__list" aria-label="Stored projects">
        {sorted.length === 0 && <li className="eth-project-mgr__empty">No stored projects</li>}
        {sorted.map((p) => {
          const current = p.id === currentId;
          return (
            <li key={p.id} className="eth-project-mgr__item" data-current={current || undefined} aria-current={current || undefined}>
              {renaming?.id === p.id ? (
                <form className="eth-project-mgr__rename" onSubmit={commitRename}>
                  <input
                    className="eth-project-mgr__input"
                    aria-label={`New name for ${p.name}`}
                    autoFocus
                    value={renaming.name}
                    onChange={(e) => setRenaming({ id: p.id, name: e.target.value })}
                    onBlur={commitRename}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        e.stopPropagation(); // don't close the popover
                        setRenaming(null);
                      }
                    }}
                  />
                </form>
              ) : (
                <span className="eth-project-mgr__name" title={p.name}>
                  {p.name}
                  {current && <span className="eth-project-mgr__badge">open</span>}
                </span>
              )}
              <span className="eth-project-mgr__date">{formatModified(p.modified_ms)}</span>
              <span className="eth-project-mgr__actions">
                <Button
                  size="sm"
                  disabled={busy || current}
                  aria-label={`Open ${p.name}`}
                  onClick={() => void run(cmd("Project", { type: "Open", id: p.id }), true)}
                >
                  Open
                </Button>
                <Button size="sm" disabled={busy} aria-label={`Rename ${p.name}`} onClick={() => setRenaming({ id: p.id, name: p.name })}>
                  Rename
                </Button>
                <Button size="sm" disabled={busy} aria-label={`Duplicate ${p.name}`} onClick={() => duplicate(p)}>
                  Duplicate
                </Button>
                <Button
                  size="sm"
                  disabled={busy || current}
                  title={current ? "The open project can't be deleted" : undefined}
                  aria-label={confirmDelete === p.id ? `Confirm delete ${p.name}` : `Delete ${p.name}`}
                  className={confirmDelete === p.id ? "eth-project-mgr__danger" : undefined}
                  onClick={() => remove(p)}
                  onBlur={() => setConfirmDelete((id) => (id === p.id ? null : id))}
                >
                  {confirmDelete === p.id ? "Confirm" : "Delete"}
                </Button>
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
