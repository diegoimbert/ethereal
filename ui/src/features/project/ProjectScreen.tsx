import { useEffect, useMemo, useState, type FormEvent, type MouseEvent } from "react";
import { MoreHorizontal, Plus } from "lucide-react";
import type { ProjectSummary } from "@/generated";
import { errorMessage, type EngineCommands } from "@/features/transport-bar/engine";
import { Button, Dialog, IconButton, openContextMenu, TextInput, type ContextMenuEntry } from "@/kit";
import { ProjectScale } from "@/features/scale/ProjectScale";
import { useProjectStore } from "@/state";
import { cmd, newProjectId, type EngineTransport } from "@/transport";
import { copyName, formatModified, sortProjects, uniqueName } from "./projectNames";
import { useProjectScreen } from "./screenStore";
import { ShareBadges } from "./ShareBadges";
import { copyInviteLink, makePrivateCopy, reconnectCopy, stopSharing } from "./shareActions";

/**
 * The project screen: a modal over the whole app, shown on launch and from the Projects
 * button. The open project (on launch: the previous one) with its name editable, a big
 * "New project" button (asks for a name), and the other stored projects to open, rename,
 * duplicate or delete. Escape or a click outside continues with the open project.
 */
export function ProjectScreen({ commands }: { commands: EngineCommands }) {
  const open = useProjectScreen((s) => s.open);
  const hide = useProjectScreen((s) => s.hide);
  const [naming, setNaming] = useState(false);
  const close = () => {
    hide();
    setNaming(false);
  };
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
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  // Outcome of a sharing action ("Link copied", or why it failed).
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);

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

  /** Run a sharing action (they may open the project first), reporting its outcome. */
  const share = async (action: (t: EngineTransport) => Promise<unknown>, done?: string, close = false) => {
    if (!commands.transport) return;
    setBusy(true);
    setNotice(null);
    try {
      await action(commands.transport);
      if (done) setNotice({ text: done, error: false });
      if (close) onDone();
    } catch (err) {
      setNotice({ text: errorMessage(err), error: true });
    } finally {
      setBusy(false);
    }
  };

  /** The sharing entries of a project's menu (SHARING.md §8.5). */
  const shareActions = (p: ProjectSummary): ShareAction[] => {
    const s = p.share;
    if (!s) return [];
    if (s.role === "Host") {
      if (!s.active) return [];
      return [
        { label: "Copy invite link", onSelect: () => void share((t) => copyInviteLink(t, p.id), "Link copied") },
        { label: "Stop sharing…", danger: true, onSelect: () => setConfirm({ id: p.id, kind: "stop" }) },
      ];
    }
    const actions: ShareAction[] = [];
    if (s.active) actions.push({ label: "Reconnect", onSelect: () => void share((t) => reconnectCopy(t, p.id), undefined, true) });
    actions.push({ label: "Make a private copy…", onSelect: () => setConfirm({ id: p.id, kind: "detach" }) });
    return actions;
  };

  const menu = (e: MouseEvent, p: ProjectSummary) => {
    const sharing = shareActions(p);
    const entries: ContextMenuEntry[] = [
      { label: "Open", onSelect: () => void run(cmd("Project", { type: "Open", id: p.id })).then((r) => r && onDone()) },
      { label: "Rename", onSelect: () => setRenaming({ id: p.id, name: p.name }) },
      {
        label: "Duplicate",
        onSelect: () => void run(cmd("Project", { type: "Duplicate", id: p.id, new_id: newProjectId(), name: copyName(p.name, projects) })),
      },
      ...(sharing.length > 0 ? (["separator", ...sharing] as ContextMenuEntry[]) : []),
      "separator",
      { label: "Delete…", danger: true, onSelect: () => setConfirm({ id: p.id, kind: "delete" }) },
    ];
    openContextMenu(e, entries);
  };

  /** "Stop sharing" / "Make private" inline confirmation for project `p`. */
  const shareConfirm = (p: ProjectSummary, kind: "stop" | "detach") => {
    const people = (p.share?.participants ?? []).map((x) => x.name);
    const text =
      kind === "stop"
        ? `Stop sharing “${p.name}”? ${people.length > 0 ? `${listNames(people)} keep an offline copy. ` : ""}Links stop working.`
        : `Make “${p.name}” private? It stops syncing with ${p.share?.host_name || "the host"}.`;
    const label = kind === "stop" ? "Stop sharing" : "Make private";
    return (
      <span className="eth-project-screen__confirm">
        <span className="eth-project-screen__name" title={text}>
          {text}
        </span>
        <Button size="sm" onClick={() => setConfirm(null)}>
          Cancel
        </Button>
        <Button
          size="sm"
          className="eth-project-screen__danger"
          disabled={busy}
          aria-label={`Confirm ${label.toLowerCase()} ${p.name}`}
          onClick={() => {
            setConfirm(null);
            void share((t) => (kind === "stop" ? stopSharing(t, p.id) : makePrivateCopy(t, p.id)));
          }}
        >
          {label}
        </Button>
      </span>
    );
  };

  const currentSummary = current ? projects.find((p) => p.id === current.id) : undefined;
  const currentActions = currentSummary ? shareActions(currentSummary) : [];

  return (
    <div className="eth-project-screen__body">
      {current && (
        <section className="eth-project-screen__current" aria-label="Open project">
          <span className="eth-project-screen__label eth-project-screen__heading">
            Open project
            {currentSummary && <ShareBadges project={currentSummary} />}
          </span>
          <CurrentName key={current.id} name={current.settings.name} onRename={(name) => rename(current.id, name)} />
          <Button variant="primary" onClick={onDone}>
            Continue
          </Button>
          {currentSummary && confirm?.id === current.id && confirm.kind !== "delete" ? (
            <div className="eth-project-screen__share-actions">{shareConfirm({ ...currentSummary, name: current.settings.name }, confirm.kind)}</div>
          ) : (
            currentActions.length > 0 && (
              <div className="eth-project-screen__share-actions" role="group" aria-label="Sharing">
                {currentActions.map((a) => (
                  <Button key={a.label} size="sm" tone={a.danger ? "danger" : "ghost"} disabled={busy} onClick={a.onSelect}>
                    {a.label}
                  </Button>
                ))}
              </div>
            )
          )}
          <ProjectScale send={send} />
        </section>
      )}

      <button type="button" className="eth-project-screen__new" onClick={onNew} disabled={busy}>
        <Plus aria-hidden />
        New project
      </button>

      <ErrorLine commands={commands} />
      {notice && (
        <button
          type="button"
          className={notice.error ? "eth-project__error" : "eth-project-screen__notice"}
          role={notice.error ? "alert" : "status"}
          title="Dismiss"
          onClick={() => setNotice(null)}
        >
          {notice.text}
        </button>
      )}

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
                ) : confirm?.id === p.id && confirm.kind !== "delete" ? (
                  shareConfirm(p, confirm.kind)
                ) : confirm?.id === p.id ? (
                  <span className="eth-project-screen__confirm">
                    <span className="eth-project-screen__name">Delete “{p.name}”?</span>
                    <Button size="sm" onClick={() => setConfirm(null)}>
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      className="eth-project-screen__danger"
                      disabled={busy}
                      aria-label={`Confirm delete ${p.name}`}
                      onClick={() => {
                        setConfirm(null);
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
                    <ShareBadges project={p} />
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

/** A pending inline confirmation in the project screen. */
interface Confirm {
  id: string;
  kind: "delete" | "stop" | "detach";
}

interface ShareAction {
  label: string;
  danger?: boolean;
  onSelect(): void;
}

/** "Ada", "Ada and Tom", "Ada, Tom and Kim". */
function listNames(names: ReadonlyArray<string>): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
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

  const create = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    const reply = await send(cmd("Project", { type: "Create", id: newProjectId(), name: uniqueName(name || suggested, projects) }));
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
