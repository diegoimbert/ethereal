import { useEffect, useMemo, useRef, useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { GitFork, MoreHorizontal, Plus, Search } from "lucide-react";
import type { ProjectSummary } from "@/generated";
import { errorMessage, useOptionalConnection, type EngineCommands } from "@/features/transport-bar/engine";
import { NO_AUTOCORRECT, Badge, Button, Dialog, DismissibleError, IconButton, openContextMenu, TextInput, type ContextMenuEntry } from "@/kit";
import { useRecoveryShown } from "@/features/versions/store";
import { DiscordLink } from "@/features/community";
import { notify } from "@/features/notifications";
import { newProjectCommand, ProjectTemplatePicker, type ProjectTemplateChoice } from "@/features/templates";
import { useProjectStore } from "@/state";
import { cmd, listProjects, newProjectId, type EngineTransport } from "@/transport";
import { moveToBin, saveProjectAs } from "./actions";
import { useExportProject } from "./exportProjectStore";
import { guardLeave } from "./leaveGuard";
import { copyName, dateName, formatModified, sortProjects, uniqueName } from "./projectNames";
import { createDraftProject, isDraft } from "./drafts";
import { useProjectScreen } from "./screenStore";
import { GetStarted } from "./GetStarted";
import { APP_VERSION } from "@/config/version";
import { splitLocalCopy } from "./sessionMarks";
import { t as translate, Trans, useT } from "@/i18n";
import { contributions, useContributions, type ProjectRowAction } from "@/ext";
import { ProjectShelves } from "./ProjectShelves";

/**
 * The project picker ("Open project"): a modal listing the stored projects to
 * open (open without plugins, rename, duplicate, export or delete from a row's menu), with a
 * "New project" button. It is shown on launch (nothing opens by itself) and from
 * the project menu; dismissing it with no project open lands in a new draft project
 * (`drafts.ts`: named after today's date, kept only once changed). The project's own actions
 * live in the top bar's project menu (`index.tsx`). It also hosts the "New project" form
 * (templates) and "Save as" (command palette). The crash-recovery dialog goes first.
 *
 * At launch it can show before the engine is up (`launch.ts`): its actions are disabled
 * and the recent projects are placeholder rows (as many as last time) until launch is done.
 */
export function ProjectScreen({ commands }: { commands: EngineCommands }) {
  const t = useT();
  const recovery = useRecoveryShown((s) => s.shown);
  const open = useProjectScreen((s) => s.open) && !recovery;
  const mode = useProjectScreen((s) => s.mode);
  const hide = useProjectScreen((s) => s.hide);
  const setMode = useProjectScreen((s) => s.setMode);
  const home = () => setMode("home");
  // Dismissed with no project open: a new draft project (kept once something changes). While
  // launching, launch makes that draft.
  const dismiss = () => {
    hide();
    if (useProjectScreen.getState().launching) return;
    if (!useProjectStore.getState().project && commands.transport) void createDraftProject(commands.transport).catch(() => undefined);
  };
  return (
    <Dialog
      open={open}
      onClose={dismiss}
      title={mode === "saveAs" ? t("project.screen.save_as") : mode === "new" ? t("project.new_project") : t("project.screen.open")}
      // Home is the welcome screen: wide, its own header (the dialog title stays its name), no
      // close button (Escape or a click outside leaves it).
      closeButton={mode !== "home"}
      className={mode === "home" ? "eth-project-screen eth-project-screen--welcome" : "eth-project-screen"}
    >
      {mode === "new" ? (
        <NewProject commands={commands} onBack={home} onDone={hide} />
      ) : mode === "saveAs" ? (
        <NameForm
          commands={commands}
          label={t("project.save_as.label")}
          inputLabel={t("project.save_as.input")}
          submit={t("project.save_as.submit")}
          leave={t("project.leave.save")}
          suggested={(projects) => copyName(useProjectStore.getState().project?.settings.name ?? translate("project.names.untitled"), projects)}
          run={(name) => (commands.transport ? saveProjectAs(commands.transport, name) : Promise.resolve(undefined))}
          onBack={home}
          onDone={hide}
        />
      ) : (
        <Home commands={commands} onDone={hide} />
      )}
    </Dialog>
  );
}

function ErrorLine({ commands, extra, clearExtra }: { commands: EngineCommands; extra?: string | null; clearExtra?: () => void }) {
  const { error, clearError } = commands;
  const shown = extra ?? error;
  if (!shown) return null;
  return (
    <DismissibleError
      className="eth-project__error"
      onDismiss={() => {
        clearError();
        clearExtra?.();
      }}
    >
      {shown}
    </DismissibleError>
  );
}

/**
 * A project's badges in the list or the current section: "Local copy" (the backup kept when a
 * differing stored copy was replaced).
 */
function ProjectBadges({ project, name = project?.name }: { project: ProjectSummary | undefined; name?: string }) {
  const t = useT();
  const extra = useContributions("project.badges");
  const localCopy = name !== undefined && splitLocalCopy(name).localCopy;
  return (
    <>
      {localCopy && (
        <span title={t("project.badge.local_copy_hint")}>
          <Badge tone="warn">{t("project.badge.local_copy")}</Badge>
        </span>
      )}
      {/* Extensions' badges (`@/ext` "project.badges"). */}
      {project && extra.map((Extra, i) => <Extra key={i} project={project} />)}
    </>
  );
}

/** The brand, never translated. */
const WORDMARK = "ethereal"; // i18n-ignore

/** localStorage: how many recent projects the picker listed last time (its placeholder rows at launch). */
export const RECENT_COUNT_KEY = "eth.project.recent-count";
/** At most this many placeholder rows (the list scrolls past a few anyway). */
const MAX_BONES = 12;

function readRecentCount(): number {
  try {
    const n = Number(localStorage.getItem(RECENT_COUNT_KEY));
    return Number.isFinite(n) ? Math.max(0, Math.min(MAX_BONES, Math.floor(n))) : 0;
  } catch {
    return 0;
  }
}

function writeRecentCount(n: number): void {
  try {
    localStorage.setItem(RECENT_COUNT_KEY, String(n));
  } catch {
    // Storage unavailable: no placeholder rows next time.
  }
}

/**
 * The recent projects: a search, then the list (open on a click; a row's menu: open without
 * plugins, rename, duplicate, export, delete, extensions' actions), placeholder rows while
 * launching. The welcome screen's right column, and the project menu's "Open project" submenu.
 */
export function RecentProjects({
  commands,
  onDone,
  ready = true,
  focusSearch = false,
  showEmpty = false,
  extraError,
  clearExtraError,
  shelves = false,
}: {
  commands: EngineCommands;
  /** A project opened (the screen / menu closes). */
  onDone(): void;
  /** Launch is done (else placeholder rows). */
  ready?: boolean;
  /** Focus the search once ready (unless typing elsewhere). */
  focusSearch?: boolean;
  /** Say so when there is no other project (the project menu's submenu; the screen has more). */
  showEmpty?: boolean;
  /** An error of the surroundings to show on the error line too (e.g. "new project" failed). */
  extraError?: string | null;
  clearExtraError?(): void;
  /** Also the archived projects and the bin (the project picker; not the menu's submenu). */
  shelves?: boolean;
}) {
  const t = useT();
  const { send, transport } = commands;
  const [bones] = useState(readRecentCount);
  const projects = useProjectStore((s) => s.projects);
  const current = useProjectStore((s) => s.project);
  // Not the open project, nor untouched drafts, nor archived ones (`ProjectShelves`).
  const others = useMemo(() => sortProjects(projects).filter((p) => p.id !== current?.id && !isDraft(p.id) && !p.archived), [projects, current?.id]);
  // The search over the recent projects (focused when the screen opens).
  const [query, setQuery] = useState("");
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? others.filter((p) => p.name.toLowerCase().includes(q)) : others;
  }, [others, query]);
  const searchRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    // After the dialog focused itself; once ready, unless the user is typing somewhere else.
    if (!ready || !focusSearch) return;
    const timer = setTimeout(() => {
      if (!(document.activeElement instanceof HTMLInputElement)) searchRef.current?.querySelector("input")?.focus();
    }, 0);
    return () => clearTimeout(timer);
  }, [ready, focusSearch]);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const listSections = useContributions("project.listSection");
  // A pending extension row action's confirmation (`@/ext` "project.rowActions").
  const [confirmAction, setConfirmAction] = useState<{ id: string; action: ProjectRowAction } | null>(null);
  // Failures of actions run outside `send` (new project, extensions' row actions).
  const [error, setError] = useState<string | null>(null);

  // How many rows to hold for the list next launch.
  useEffect(() => {
    if (ready) writeRecentCount(others.length);
  }, [ready, others.length]);

  // Refresh the list when shown, once the engine is up (it is also kept current by
  // `Project::ListChanged`).
  useEffect(() => {
    if (!ready || !transport) return;
    let active = true;
    listProjects(transport).then(
      (reply) => {
        if (active && reply.type === "Projects") useProjectStore.getState().setProjects(reply.projects);
      },
      (e: unknown) => active && setError(errorMessage(e)),
    );
    return () => {
      active = false;
    };
  }, [transport, ready]);

  const run = async (command: Parameters<typeof send>[0]) => {
    setBusy(true);
    const reply = await send(command);
    setBusy(false);
    return reply;
  };

  /** Run an action on the transport, showing its failure on the error line. */
  const act = async <T,>(action: (tr: EngineTransport) => Promise<T>): Promise<T | undefined> => {
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

  const openProject = (id: string, safe = false) => guardLeave(() => run(cmd("Project", { type: safe ? "OpenSafe" : "Open", id })).then((r) => r && onDone()));

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

  /** Run an extension's row action (opening its project first when asked): a toast when it says so. */
  const runRowAction = async (p: ProjectSummary, a: ProjectRowAction) => {
    const go = async () => {
      const ok = await act(async (tr) => {
        await a.run(tr);
        return true;
      });
      if (!ok) return;
      if (a.done) notify("Info", a.done);
      if (a.close) onDone();
    };
    if (a.openFirst && p.id !== current?.id) await guardLeave(go);
    else await go();
  };

  const menu = (e: MouseEvent, p: ProjectSummary) => {
    const extra = contributions("project.rowActions").flatMap((f) => [...f(p)]);
    const entries: ContextMenuEntry[] = [
      { label: t("project.actions.open"), onSelect: () => void openProject(p.id) },
      // Plugin devices stay bypassed placeholders until "Load plugins".
      { label: t("project.actions.open_safe"), onSelect: () => void openProject(p.id, true) },
      { label: t("project.actions.rename"), onSelect: () => setRenaming({ id: p.id, name: p.name }) },
      {
        label: t("project.row.duplicate"),
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
        label: t("project.row.export"),
        onSelect: () => useExportProject.getState().show(p.id, p.name),
      },
      ...(extra.length > 0
        ? ([
            "separator",
            ...extra.map((a) => ({
              label: a.label,
              danger: a.danger,
              onSelect: () => (a.confirm ? setConfirmAction({ id: p.id, action: a }) : void runRowAction(p, a)),
            })),
          ] as ContextMenuEntry[])
        : []),
      "separator",
      { label: t("project.row.archive"), onSelect: () => void run(cmd("Project", { type: "SetArchived", id: p.id, archived: true })) },
      // Restorable (the toast, the bin); deleted for good from the bin (`ProjectShelves`).
      { label: t("project.row.move_to_bin"), danger: true, onSelect: () => void act((tr) => moveToBin(tr, p)) },
    ];
    openContextMenu(e, entries);
  };

  return (
    <div className="eth-project-screen__body">
      <ErrorLine
        commands={commands}
        extra={error ?? extraError}
        clearExtra={() => {
          setError(null);
          clearExtraError?.();
        }}
      />

      {!ready && bones > 0 && (
        <section className="eth-project-screen__recent" aria-label={t("project.recent.title")} aria-busy="true">
          <div className="eth-project-screen__recent-head">
            <span className="eth-project-screen__label">{t("project.recent.title")}</span>
            <span className="eth-project-screen__search">
              <Search className="eth-project-screen__search-icon" aria-hidden />
              <TextInput
                size="sm"
                type="search"
                {...NO_AUTOCORRECT}
                aria-label={t("project.recent.search")}
                placeholder={t("project.recent.search")}
                disabled
                value=""
                readOnly
              />
            </span>
          </div>
          <ul className="eth-project-screen__list" data-testid="project-list-skeleton">
            {Array.from({ length: bones }, (_, i) => (
              <li key={i} className="eth-project-screen__item eth-project-screen__item--bone" aria-hidden>
                <span className="eth-project-screen__bone" />
              </li>
            ))}
          </ul>
        </section>
      )}
      {ready && others.length === 0 && showEmpty && <span className="eth-project-screen__empty">{t("project.recent.none")}</span>}
      {ready && others.length > 0 && (
        <section className="eth-project-screen__recent" aria-label={t("project.recent.title")}>
          <div className="eth-project-screen__recent-head">
            <span className="eth-project-screen__label">{t("project.recent.title")}</span>
            <span className="eth-project-screen__search" ref={searchRef}>
              <Search className="eth-project-screen__search-icon" aria-hidden />
              <TextInput
                size="sm"
                type="search"
                {...NO_AUTOCORRECT}
                aria-label={t("project.recent.search")}
                placeholder={t("project.recent.search")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  // Enter opens the first match.
                  if (e.key === "Enter" && shown[0]) {
                    e.preventDefault();
                    void openProject(shown[0].id);
                  }
                }}
              />
            </span>
          </div>
          {shown.length === 0 && (
            <span className="eth-project-screen__empty">
              <Trans k="project.recent.no_match" values={{ query: <span className="eth-user-text">{query.trim()}</span> }} />
            </span>
          )}
          <ul className="eth-project-screen__list" aria-label={t("project.recent.stored")}>
            {shown.map((p) => (
              <li key={p.id} className="eth-project-screen__item" onContextMenu={(e) => menu(e, p)}>
                {renaming?.id === p.id ? (
                  <form className="eth-project-screen__rename" onSubmit={commitRename}>
                    <TextInput
                      aria-label={t("project.actions.rename_input", { name: p.name })}
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
                ) : confirmAction?.id === p.id && confirmAction.action.confirm ? (
                  <span className="eth-project-screen__confirm">
                    <span className="eth-project-screen__name" title={confirmAction.action.confirm.title}>
                      {confirmAction.action.confirm.text}
                    </span>
                    <Button size="sm" onClick={() => setConfirmAction(null)}>
                      {t("project.actions.cancel")}
                    </Button>
                    <Button
                      size="sm"
                      className="eth-project-screen__danger"
                      disabled={busy}
                      aria-label={confirmAction.action.confirm.buttonLabel}
                      onClick={() => {
                        const a = confirmAction.action;
                        setConfirmAction(null);
                        void runRowAction(p, a);
                      }}
                    >
                      {confirmAction.action.confirm.button}
                    </Button>
                  </span>
                ) : (
                  <button
                    type="button"
                    className="eth-project-screen__open"
                    aria-label={t("project.actions.open_named", { name: p.name })}
                    disabled={busy}
                    onClick={() => void openProject(p.id)}
                  >
                    <span className="eth-project-screen__name eth-user-text" title={p.name}>
                      {splitLocalCopy(p.name).base}
                    </span>
                    {p.forked_from && (
                      <span className="eth-project-screen__fork" title={t("project.fork.from", { name: p.forked_from.name })}>
                        <GitFork aria-hidden />
                        <span className="eth-user-text">{p.forked_from.name}</span>
                      </span>
                    )}
                    <ProjectBadges project={p} />
                    {/* A date in the user's language (Intl), not catalog text. */}
                    <span className="eth-project-screen__date" data-i18n-exempt>
                      {formatModified(p.modified_ms)}
                    </span>
                  </button>
                )}
                <IconButton
                  size="sm"
                  tone="ghost"
                  label={t("project.actions.more", { name: p.name })}
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
      {/* Extensions' sections (`@/ext` "project.listSection"), e.g. the projects in the cloud. */}
      {listSections.map((Section, i) => (
        <Section key={i} query={query} ready={ready} onDone={onDone} />
      ))}
      {shelves && ready && <ProjectShelves commands={commands} openProject={(id) => void openProject(id)} setError={setError} />}
    </div>
  );
}

function Home({ commands, onDone }: { commands: EngineCommands; onDone(): void }) {
  const t = useT();
  const { transport } = commands;
  // Launch isn't done (the engine is loading): the actions wait, the list is placeholders.
  const ready = !useProjectScreen((s) => s.launching);
  const engineFailed = useOptionalConnection()?.status === "error";
  // Why the picker is shown (the address bar's project isn't stored here).
  const notice = useProjectScreen((s) => s.notice);
  const welcomeActions = useContributions("project.welcomeActions");
  const [busy, setBusy] = useState(false);
  // Failures of actions run outside `send` (new project, extensions' welcome actions).
  const [error, setError] = useState<string | null>(null);

  /** Run an action on the transport, showing its failure on the error line. */
  const act = async <T,>(action: (tr: EngineTransport) => Promise<T>): Promise<T | undefined> => {
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

  // The welcome screen: the wordmark and version with its actions (discord, extensions', then
  // "new project" at the very right), then Get started | the recent projects.
  return (
    <div className="eth-welcome">
      <header className="eth-welcome__brand">
        <span className="eth-welcome__wordmark" data-i18n-exempt>
          {WORDMARK}
        </span>
        {/* A dev build (the Vite dev server) says so; a release shows its version alone. */}
        <span className="eth-welcome__version">
          v{APP_VERSION}
          {/* i18n-ignore */}
          {import.meta.env.DEV && <span data-i18n-exempt> (dev)</span>}
        </span>
        {!ready && (
          <span className="eth-welcome__loading" role="status" data-testid="project-screen-loading">
            {engineFailed ? t("project.welcome.engine_failed") : t("project.welcome.loading")}
          </span>
        )}
        <span className="eth-welcome__actions">
          <DiscordLink variant="button" />
          {/* Extensions' buttons (`@/ext` "project.welcomeActions"). */}
          {welcomeActions.map((Action, i) => (
            <Action key={i} busy={busy} ready={ready} onDone={onDone} />
          ))}
          {/* The main action, at the very end of the row. */}
          <Button
            size="sm"
            tone="accent"
            data-testid="welcome-new-project"
            disabled={busy || !ready}
            onClick={() =>
              void guardLeave(async () => {
                if (await act((tr) => createDraftProject(tr).then(() => true))) onDone();
              }, t("project.leave.create"))
            }
          >
            <Plus aria-hidden />
            {t("project.new_project")}
          </Button>
        </span>
      </header>
      {notice && (
        <p className="eth-welcome__notice" role="status" data-testid="project-screen-notice">
          {t(notice)}
        </p>
      )}
      <div className="eth-welcome__columns">
        <GetStarted commands={commands} onStarted={onDone} />
        <RecentProjects commands={commands} onDone={onDone} ready={ready} focusSearch shelves extraError={error} clearExtraError={() => setError(null)} />
      </div>
    </div>
  );
}

/** The "New project" form: a name and the project template to start from (templates). */
function NewProject({ commands, onBack, onDone }: { commands: EngineCommands; onBack(): void; onDone(): void }) {
  const t = useT();
  // templates: the project template to start from (`undefined`: the default one).
  const [template, setTemplate] = useState<ProjectTemplateChoice | undefined>(undefined);
  return (
    <NameForm
      commands={commands}
      label={t("project.new.label")}
      inputLabel={t("project.new.input")}
      submit={t("project.new.submit")}
      leave={t("project.leave.create")}
      suggested={(projects) => dateName(projects)}
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
  const t = useT();
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
        <Button onClick={onBack}>{t("project.form.back")}</Button>
        <Button type="submit" variant="primary" disabled={busy}>
          {submit}
        </Button>
      </div>
    </form>
  );
}
