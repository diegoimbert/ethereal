import { useEffect, useMemo, useRef, useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { History, MoreHorizontal, Plus } from "lucide-react";
import type { ProjectSummary } from "@/generated";
import { errorMessage, type EngineCommands } from "@/features/transport-bar/engine";
import { Badge, Button, Dialog, IconButton, openContextMenu, TextInput } from "@/kit";
import { ProjectScale } from "@/features/scale/ProjectScale";
import { openVersions } from "@/features/versions/store";
import { newProjectCommand, ProjectTemplatePicker, useTemplateDialog, type ProjectTemplateChoice } from "@/features/templates";
import { useProjectStore } from "@/state";
import { cmd, newProjectId, type EngineTransport } from "@/transport";
import { closeAndDelete, duplicateProject, exportProject, importProject, saveProjectAs } from "./actions";
import { guardLeave } from "./leaveGuard";
import { copyName, formatModified, sortProjects, uniqueName } from "./projectNames";
import { useProjectScreen } from "./screenStore";
import { splitLocalCopy, useSessionOf } from "./sessionMarks";

/**
 * The project screen: a modal over the whole app, shown on launch and from the Projects
 * button. The open project (on launch: the previous one) with its name editable and its
 * actions (rename, save as, duplicate, export, import, delete), a big "New project" button
 * (asks for a name), and the other stored projects to open, rename, duplicate or delete.
 * Escape or a click outside continues with the open project. In a collaboration session,
 * opening/creating/saving as another project asks first (`guardLeave`).
 */
export function ProjectScreen({ commands }: { commands: EngineCommands }) {
  const open = useProjectScreen((s) => s.open);
  const mode = useProjectScreen((s) => s.mode);
  const hide = useProjectScreen((s) => s.hide);
  const setMode = useProjectScreen((s) => s.setMode);
  const home = () => setMode("home");
  return (
    <Dialog open={open} onClose={hide} title={mode === "saveAs" ? "Save as" : "Projects"} className="eth-project-screen">
      {mode === "new" ? (
        <NewProject commands={commands} onBack={home} onDone={hide} />
      ) : mode === "saveAs" ? (
        <NameForm
          commands={commands}
          label="Save a copy of this project as"
          inputLabel="Save as name"
          submit="Save"
          leave="Leave & save"
          suggested={(projects) => copyName(useProjectStore.getState().project?.settings.name ?? "Untitled", projects)}
          run={(name) => (commands.transport ? saveProjectAs(commands.transport, name) : Promise.resolve(undefined))}
          onBack={home}
          onDone={hide}
        />
      ) : (
        <Home commands={commands} onNew={() => setMode("new")} onDone={hide} />
      )}
    </Dialog>
  );
}

function ErrorLine({ commands, extra, clearExtra }: { commands: EngineCommands; extra?: string | null; clearExtra?: () => void }) {
  const { error, clearError } = commands;
  const shown = extra ?? error;
  if (!shown) return null;
  return (
    <button
      type="button"
      className="eth-project__error"
      role="alert"
      title="Dismiss"
      onClick={() => {
        clearError();
        clearExtra?.();
      }}
    >
      {shown}
    </button>
  );
}

/** "Local copy" / "Collab" badges of a project in the list or the current section. */
function ProjectBadges({ id, name }: { id: string; name: string }) {
  const session = useSessionOf(id);
  const { localCopy } = splitLocalCopy(name);
  return (
    <>
      {localCopy && (
        <span title="The version you had before joining a collaboration session, kept when the session replaced it">
          <Badge tone="warn">Local copy</Badge>
        </span>
      )}
      {session && (
        <span title={`Last used in the collaboration session “${session}”`}>
          <Badge tone="accent">Collab</Badge>
        </span>
      )}
    </>
  );
}

function Home({ commands, onNew, onDone }: { commands: EngineCommands; onNew(): void; onDone(): void }) {
  const { send, transport } = commands;
  const projects = useProjectStore((s) => s.projects);
  const current = useProjectStore((s) => s.project);
  const others = useMemo(() => sortProjects(projects).filter((p) => p.id !== current?.id), [projects, current?.id]);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  // Failures of actions run outside `send` (export, import, duplicate, close and delete).
  const [error, setError] = useState<string | null>(null);

  // Refresh the list when shown (it is also kept current by `Project::ListChanged`).
  useEffect(() => {
    let active = true;
    void send(cmd("Project", { type: "List" })).then((reply) => {
      if (active && reply?.type === "Projects") useProjectStore.getState().setProjects(reply.projects);
    });
    return () => {
      active = false;
    };
  }, [send]);

  const run = async (command: Parameters<typeof send>[0]) => {
    setBusy(true);
    const reply = await send(command);
    setBusy(false);
    return reply;
  };

  /** Run an action on the transport, showing its failure on the error line. */
  const act = async <T,>(action: (t: EngineTransport) => Promise<T>): Promise<T | undefined> => {
    if (!transport) return undefined;
    setBusy(true);
    setError(null);
    try {
      return await action(transport);
    } catch (e) {
      setError(errorMessage(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  };

  const openProject = (id: string) => guardLeave(() => run(cmd("Project", { type: "Open", id })).then((r) => r && onDone()));

  const rename = (id: string, name: string) => {
    const trimmed = name.trim();
    const before = id === current?.id ? current.settings.name : projects.find((p) => p.id === id)?.name;
    if (!trimmed || trimmed === before) return;
    void run(cmd("Project", { type: "Rename", id, name: trimmed }));
  };

  const commitRename = (e?: FormEvent) => {
    e?.preventDefault();
    if (!renaming) return;
    setRenaming(null);
    rename(renaming.id, renaming.name);
  };

  const importBundle = async () => {
    const summary = await act((t) => importProject(t));
    if (summary) await openProject(summary.id);
  };

  const menu = (e: MouseEvent, p: ProjectSummary) =>
    openContextMenu(e, [
      { label: "Open", onSelect: () => void openProject(p.id) },
      {
        label: "Rename",
        onSelect: () => setRenaming({ id: p.id, name: p.name }),
      },
      {
        label: "Duplicate",
        onSelect: () =>
          void run(
            cmd("Project", {
              type: "Duplicate",
              id: p.id,
              new_id: newProjectId(),
              name: copyName(p.name, projects),
            }),
          ),
      },
      {
        label: "Export…",
        onSelect: () => void act((t) => exportProject(t, p.id, p.name)),
      },
      "separator",
      {
        label: "Delete…",
        danger: true,
        onSelect: () => setConfirmDelete(p.id),
      },
    ]);

  return (
    <div className="eth-project-screen__body">
      {current && (
        <section className="eth-project-screen__current" aria-label="Open project">
          <span className="eth-project-screen__label eth-project-screen__heading">
            Open project
            <ProjectBadges id={current.id} name={current.settings.name} />
          </span>
          <CurrentName key={current.id} name={current.settings.name} onRename={(name) => rename(current.id, name)} />
          <Button variant="primary" onClick={onDone}>
            Continue
          </Button>
          {/* templates: save the open project as a project template. */}
          <Button
            tone="ghost"
            size="sm"
            className="eth-project-screen__save-template"
            onClick={() => useTemplateDialog.getState().open({ type: "save-project", name: current.settings.name })}
          >
            Save as template…
          </Button>
          {confirmDelete === current.id ? (
            <div className="eth-project-screen__confirm eth-project-screen__toolbar" role="group" aria-label="Confirm delete">
              <span className="eth-project-screen__name">Delete “{current.settings.name}”? It closes first; this can&apos;t be undone.</span>
              <Button size="sm" onClick={() => setConfirmDelete(null)}>
                Cancel
              </Button>
              <Button
                size="sm"
                tone="danger"
                disabled={busy}
                onClick={() => {
                  setConfirmDelete(null);
                  const id = current.id;
                  void guardLeave(() => act((t) => closeAndDelete(t, id)), "Leave & delete");
                }}
              >
                Close and delete
              </Button>
            </div>
          ) : (
            <div className="eth-project-screen__toolbar" role="group" aria-label="Project actions">
              <Button size="sm" tone="ghost" onClick={() => useProjectScreen.getState().rename()}>
                Rename
              </Button>
              <Button size="sm" tone="ghost" disabled={busy} onClick={() => useProjectScreen.getState().setMode("saveAs")}>
                Save as…
              </Button>
              <Button size="sm" tone="ghost" disabled={busy} onClick={() => void act((t) => duplicateProject(t, current.id, current.settings.name))}>
                Duplicate
              </Button>
              <Button size="sm" tone="ghost" disabled={busy} onClick={() => void act((t) => exportProject(t, current.id, current.settings.name))}>
                Export…
              </Button>
              <Button size="sm" tone="ghost" disabled={busy} onClick={() => void importBundle()}>
                Import…
              </Button>
              <Button size="sm" tone="ghost" className="eth-project-screen__delete" disabled={busy} onClick={() => setConfirmDelete(current.id)}>
                Delete…
              </Button>
            </div>
          )}
          <ProjectScale send={send} />
          {/* project-versions: the open project's versions (save, compare, restore). */}
          <Button
            size="sm"
            tone="ghost"
            className="eth-project-screen__versions"
            onClick={() => {
              onDone();
              openVersions();
            }}
          >
            <History aria-hidden />
            Versions…
          </Button>
        </section>
      )}

      <button type="button" className="eth-project-screen__new" onClick={onNew} disabled={busy}>
        <Plus aria-hidden />
        New project
      </button>

      <ErrorLine commands={commands} extra={error} clearExtra={() => setError(null)} />

      {others.length > 0 && (
        <section className="eth-project-screen__recent" aria-label="Recent projects">
          <span className="eth-project-screen__label">Recent projects</span>
          <ul className="eth-project-screen__list" aria-label="Stored projects">
            {others.map((p) => (
              <li key={p.id} className="eth-project-screen__item" onContextMenu={(e) => menu(e, p)}>
                {renaming?.id === p.id ? (
                  <form className="eth-project-screen__rename" onSubmit={commitRename}>
                    <TextInput
                      aria-label={`New name for ${p.name}`}
                      autoFocus
                      value={renaming.name}
                      onChange={(e) => setRenaming({ id: p.id, name: e.target.value })}
                      onBlur={() => commitRename()}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") {
                          e.stopPropagation(); // don't close the screen
                          setRenaming(null);
                        }
                      }}
                    />
                  </form>
                ) : confirmDelete === p.id ? (
                  <span className="eth-project-screen__confirm">
                    <span className="eth-project-screen__name">Delete “{p.name}”?</span>
                    <Button size="sm" onClick={() => setConfirmDelete(null)}>
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      className="eth-project-screen__danger"
                      disabled={busy}
                      aria-label={`Confirm delete ${p.name}`}
                      onClick={() => {
                        setConfirmDelete(null);
                        void run(cmd("Project", { type: "Delete", id: p.id }));
                      }}
                    >
                      Delete
                    </Button>
                  </span>
                ) : (
                  <button
                    type="button"
                    className="eth-project-screen__open"
                    aria-label={`Open ${p.name}`}
                    disabled={busy}
                    onClick={() => void openProject(p.id)}
                  >
                    <span className="eth-project-screen__name" title={p.name}>
                      {splitLocalCopy(p.name).base}
                    </span>
                    <ProjectBadges id={p.id} name={p.name} />
                    <span className="eth-project-screen__date">{formatModified(p.modified_ms)}</span>
                  </button>
                )}
                <IconButton
                  size="sm"
                  tone="ghost"
                  label={`More actions for ${p.name}`}
                  icon={<MoreHorizontal aria-hidden />}
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect();
                    menu(
                      {
                        clientX: r.left,
                        clientY: r.bottom,
                        preventDefault: () => undefined,
                        stopPropagation: () => undefined,
                      } as MouseEvent,
                      p,
                    );
                  }}
                />
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/** The open project's name, renamed on Enter or blur (Escape reverts). "Rename" focuses it. */
function CurrentName({ name, onRename }: { name: string; onRename(name: string): void }) {
  const [draft, setDraft] = useState(name);
  const [prev, setPrev] = useState(name);
  const form = useRef<HTMLFormElement>(null);
  const renameRequest = useProjectScreen((s) => s.renameRequest);
  if (prev !== name) {
    setPrev(name);
    setDraft(name);
  }
  useEffect(() => {
    if (renameRequest === 0) return;
    // After the dialog has focused itself.
    const t = setTimeout(() => {
      const input = form.current?.querySelector("input");
      input?.focus();
      input?.select();
    }, 0);
    return () => clearTimeout(t);
  }, [renameRequest]);
  return (
    <form
      ref={form}
      className="eth-project-screen__rename"
      onSubmit={(e) => {
        e.preventDefault();
        onRename(draft);
      }}
    >
      <TextInput
        size="lg"
        className="eth-project-screen__current-name"
        aria-label="Project name"
        title="Rename the project"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => onRename(draft)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && draft !== name) {
            e.stopPropagation(); // revert first; a second Escape closes
            setDraft(name);
          }
        }}
      />
    </form>
  );
}

/** The "New project" form: a name and the project template to start from (templates). */
function NewProject({ commands, onBack, onDone }: { commands: EngineCommands; onBack(): void; onDone(): void }) {
  // templates: the project template to start from (`undefined`: the default one).
  const [template, setTemplate] = useState<ProjectTemplateChoice | undefined>(undefined);
  return (
    <NameForm
      commands={commands}
      label="Name your project"
      inputLabel="New project name"
      submit="Create"
      leave="Leave & create"
      suggested={(projects) => uniqueName("Untitled", projects)}
      run={(name, projects) => commands.send(newProjectCommand(newProjectId(), uniqueName(name, projects), template))}
      onBack={onBack}
      onDone={onDone}
    >
      <ProjectTemplatePicker value={template} onChange={setTemplate} />
    </NameForm>
  );
}

/** Ask for a name, then create a project / save as (both switch projects: guarded). */
function NameForm({
  commands,
  label,
  inputLabel,
  submit,
  leave,
  suggested,
  run,
  onBack,
  onDone,
  children,
}: {
  commands: EngineCommands;
  label: string;
  inputLabel: string;
  submit: string;
  leave: string;
  suggested(projects: ReadonlyArray<ProjectSummary>): string;
  run(name: string, projects: ReadonlyArray<ProjectSummary>): Promise<unknown>;
  onBack(): void;
  onDone(): void;
  /** Extra fields under the name (the new-project form's template picker). */
  children?: ReactNode;
}) {
  const projects = useProjectStore((s) => s.projects);
  const placeholder = suggested(projects);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const go = (e: FormEvent) => {
    e.preventDefault();
    void guardLeave(async () => {
      setBusy(true);
      setError(null);
      try {
        if (await run(name.trim() || placeholder, projects)) onDone();
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setBusy(false);
      }
    }, leave);
  };

  return (
    <form className="eth-project-screen__body" onSubmit={go}>
      <label className="eth-project-screen__label" htmlFor="eth-project-name-form">
        {label}
      </label>
      <TextInput
        id="eth-project-name-form"
        size="lg"
        aria-label={inputLabel}
        autoFocus
        placeholder={placeholder}
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation(); // back to the list, not closed
            onBack();
          }
        }}
      />
      {children}
      <ErrorLine commands={commands} extra={error} clearExtra={() => setError(null)} />
      <div className="eth-project-screen__actions">
        <Button onClick={onBack}>Back</Button>
        <Button type="submit" variant="primary" disabled={busy}>
          {submit}
        </Button>
      </div>
    </form>
  );
}
