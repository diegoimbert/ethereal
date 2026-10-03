import { useEffect, useMemo, useState, type FormEvent, type MouseEvent } from "react";
import { History, MoreHorizontal, Plus } from "lucide-react";
import type { ProjectSummary } from "@/generated";
import type { EngineCommands } from "@/features/transport-bar/engine";
import { Button, Dialog, IconButton, openContextMenu, TextInput } from "@/kit";
import { ProjectScale } from "@/features/scale/ProjectScale";
import { openVersions } from "@/features/versions/store";
import { newProjectCommand, ProjectTemplatePicker, useTemplateDialog, type ProjectTemplateChoice } from "@/features/templates";
import { useProjectStore } from "@/state";
import { cmd, newProjectId } from "@/transport";
import { copyName, formatModified, sortProjects, uniqueName } from "./projectNames";
import { useProjectScreen } from "./screenStore";

/**
 * The project screen: a modal over the whole app, shown on launch and from the Projects
 * button. The open project (on launch: the previous one) with its name editable, a big
 * "New project" button (asks for a name), and the other stored projects to open, rename,
 * duplicate or delete. Escape or a click outside continues with the open project.
 */
export function ProjectScreen({ commands }: { commands: EngineCommands }) {
  const open = useProjectScreen((s) => s.open);
  const hide = useProjectScreen((s) => s.hide);
  const naming = useProjectScreen((s) => s.naming);
  const setNaming = useProjectScreen((s) => s.setNaming);
  const close = hide;
  return (
    <Dialog open={open} onClose={close} title="Projects" className="eth-project-screen">
      {naming ? (
        <NewProject commands={commands} onBack={() => setNaming(false)} onDone={close} />
      ) : (
        <Home commands={commands} onNew={() => setNaming(true)} onDone={close} />
      )}
    </Dialog>
  );
}

function ErrorLine({ commands }: { commands: EngineCommands }) {
  const { error, clearError } = commands;
  if (!error) return null;
  return (
    <button type="button" className="eth-project__error" role="alert" title="Dismiss" onClick={clearError}>
      {error}
    </button>
  );
}

function Home({ commands, onNew, onDone }: { commands: EngineCommands; onNew(): void; onDone(): void }) {
  const { send } = commands;
  const projects = useProjectStore((s) => s.projects);
  const current = useProjectStore((s) => s.project);
  const others = useMemo(() => sortProjects(projects).filter((p) => p.id !== current?.id), [projects, current?.id]);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

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

  const menu = (e: MouseEvent, p: ProjectSummary) =>
    openContextMenu(e, [
      { label: "Open", onSelect: () => void run(cmd("Project", { type: "Open", id: p.id })).then((r) => r && onDone()) },
      { label: "Rename", onSelect: () => setRenaming({ id: p.id, name: p.name }) },
      {
        label: "Duplicate",
        onSelect: () => void run(cmd("Project", { type: "Duplicate", id: p.id, new_id: newProjectId(), name: copyName(p.name, projects) })),
      },
      "separator",
      { label: "Delete…", danger: true, onSelect: () => setConfirmDelete(p.id) },
    ]);

  return (
    <div className="eth-project-screen__body">
      {current && (
        <section className="eth-project-screen__current" aria-label="Open project">
          <span className="eth-project-screen__label">Open project</span>
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

      <ErrorLine commands={commands} />

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
                    onClick={() => void run(cmd("Project", { type: "Open", id: p.id })).then((r) => r && onDone())}
                  >
                    <span className="eth-project-screen__name" title={p.name}>
                      {p.name}
                    </span>
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
                    menu({ clientX: r.left, clientY: r.bottom, preventDefault: () => undefined, stopPropagation: () => undefined } as MouseEvent, p);
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

/** The open project's name, renamed on Enter or blur (Escape reverts). */
function CurrentName({ name, onRename }: { name: string; onRename(name: string): void }) {
  const [draft, setDraft] = useState(name);
  const [prev, setPrev] = useState(name);
  if (prev !== name) {
    setPrev(name);
    setDraft(name);
  }
  return (
    <form
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

function NewProject({ commands, onBack, onDone }: { commands: EngineCommands; onBack(): void; onDone(): void }) {
  const { send } = commands;
  const projects = useProjectStore((s) => s.projects);
  const suggested = uniqueName("Untitled", projects);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  // templates: the project template to start from (`undefined`: the default one).
  const [template, setTemplate] = useState<ProjectTemplateChoice | undefined>(undefined);

  const create = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    const reply = await send(newProjectCommand(newProjectId(), uniqueName(name || suggested, projects), template));
    setBusy(false);
    if (reply) onDone();
  };

  return (
    <form className="eth-project-screen__body" onSubmit={create}>
      <label className="eth-project-screen__label" htmlFor="eth-new-project-name">
        Name your project
      </label>
      <TextInput
        id="eth-new-project-name"
        size="lg"
        aria-label="New project name"
        autoFocus
        placeholder={suggested}
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation(); // back to the list, not closed
            onBack();
          }
        }}
      />
      <ProjectTemplatePicker value={template} onChange={setTemplate} />
      <ErrorLine commands={commands} />
      <div className="eth-project-screen__actions">
        <Button onClick={onBack}>Back</Button>
        <Button type="submit" variant="primary" disabled={busy}>
          Create
        </Button>
      </div>
    </form>
  );
}
