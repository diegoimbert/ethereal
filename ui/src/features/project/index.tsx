// The app shell (ui/src/app/App.tsx) mounts `ProjectMenu`: keep it prop-less (read state via hooks).
import "./project.css";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Menu } from "lucide-react";
import { matchesAction } from "@/features/keymap";
import { TemplateDialogs } from "@/features/templates";
import { useEngineCommands, useOptionalConnection } from "@/features/transport-bar/engine";
import { DismissibleError, IconButton, Popover } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, WasmTransport } from "@/transport";
import { launch, showPickerEarly, useLaunchBookkeeping } from "./launch";
import { ExportProjectDialog } from "./ExportProjectDialog";
import { bindDrafts, deleteStaleDrafts } from "./drafts";
import { bindProjectUrl, urlProjectId } from "./projectUrl";
import { bindSaveOnHide } from "./saveOnHide";
import { ProjectMenuPanel } from "./ProjectMenuPanel";
import { ProjectScreen } from "./ProjectScreen";
import { SafeModeBanner } from "./SafeModeBanner";
import { useProjectScreen } from "./screenStore";
import { useT } from "@/i18n";

/**
 * Project menu: the menu button, the project's name and the unsaved-changes dot. The button
 * opens the project menu (`ProjectMenuPanel`: new, open, save as template, scale,
 * import / export, versions, delete); "Open project…" (and launch) show the project picker
 * (`ProjectScreen`). A click on the name renames the project in place.
 *
 * The project saves itself: `AUTOSAVE_MS` after the last change (each edit restarts the
 * wait, so a burst of edits is one save). Ctrl/Cmd+S saves at once. Autosaves say so
 * (`autosave: true`): the engine then re-reads only the plugins whose state changed and
 * writes in the background, while Ctrl/Cmd+S reads every plugin's live state. Plugin state
 * changes alone don't make the project dirty; the engine saves those on its own, slower.
 *
 * Every operation goes through the engine-side project store (`Command::Project`); the UI
 * never touches files. The list and the dirty flag are mirrored in the project store from
 * `Event::Project`.
 */
/** Autosave delay after the last change (ms). */
export const AUTOSAVE_MS = 1000;
/** How long a closing browser tab waits for its last save. */
const PAGEHIDE_SAVE_MS = 1500;

export function ProjectMenu() {
  const t = useT();
  const commands = useEngineCommands();
  const { transport, send, error, clearError } = commands;
  const name = useProjectStore((s) => s.project?.settings.name ?? null);
  const dirty = useProjectStore((s) => s.dirty);
  const open = useProjectScreen((s) => s.open);
  const disabled = !transport || name === null;
  // The project's name in place: a click (or "Rename" in the command palette) edits it.
  const [renaming, setRenaming] = useState(false);
  const renameRequest = useProjectScreen((s) => s.renameRequest);
  const [seenRequest, setSeenRequest] = useState(renameRequest);
  if (renameRequest !== seenRequest) {
    setSeenRequest(renameRequest);
    setRenaming(true);
  }
  // Draft projects (`drafts.ts`): kept once changed, deleted when left untouched.
  useEffect(() => (transport ? bindDrafts(transport) : undefined), [transport]);
  // The open project in the address bar (`/p/<slug>/<id>`, web only), Back / Forward.
  useEffect(() => (transport ? bindProjectUrl(transport) : undefined), [transport]);

  const revision = useProjectStore((s) => s.revision);
  const save = (autosave = false) => void send(cmd("Project", autosave ? { type: "Save", autosave } : { type: "Save" }));
  const saveRef = useRef(save);
  useEffect(() => {
    saveRef.current = disabled ? () => undefined : save;
  });

  // The picker in the app's first paint when launch is known to show it: the engine
  // goes on loading behind it.
  useLayoutEffect(() => {
    if (transport) showPickerEarly(transport);
  }, [transport]);
  // Launch: hosts open nothing, so once connected with no project, open the one in
  // the address bar (`/p/<slug>/<id>`), else reopen the last one if the setting allows it,
  // else show the project screen. An engine that
  // already has a project open (a remote engine, the mock) shows the screen over it.
  const connected = useOptionalConnection()?.status === "connected";
  const launched = useRef<unknown>(null);
  useEffect(() => {
    if (!connected || !transport || launched.current === transport) return;
    launched.current = transport;
    // Drafts left untouched last time go first (before the launch draft is made).
    // Not the one in the address bar (a reload of an untouched new project reopens it).
    const cleanup = deleteStaleDrafts(transport, urlProjectId(transport)).catch(() => undefined);
    if (useProjectStore.getState().project === null) {
      useProjectScreen.setState({ launchPending: false });
      void cleanup.then(() => launch(transport));
    } else {
      useProjectScreen.setState({ launching: false });
    }
  }, [connected, transport]);
  const launchPending = useProjectScreen((s) => s.launchPending);
  useEffect(() => {
    if (launchPending && name !== null) useProjectScreen.setState({ launchPending: false, open: true });
  }, [launchPending, name]);
  useLaunchBookkeeping(transport);

  // Autosave: once there are unsaved changes and no edit for AUTOSAVE_MS (every document
  // revision restarts the wait).
  useEffect(() => {
    if (!dirty || disabled) return;
    const timer = setTimeout(() => saveRef.current(true), AUTOSAVE_MS);
    return () => clearTimeout(timer);
  }, [dirty, disabled, revision]);
  // In the browser, a hidden or closed tab saves at once; a closing one waits for the
  // write (at most PAGEHIDE_SAVE_MS): its engine dies with it.
  useEffect(() => {
    if (!(transport instanceof WasmTransport)) return;
    return bindSaveOnHide(
      () => useProjectStore.getState().dirty,
      // Straight to the engine: it gets the command in the next microtask, before `drain`.
      () => void transport.send(cmd("Project", { type: "Save", autosave: true })).catch(() => undefined),
      () => void transport.drain(PAGEHIDE_SAVE_MS),
    );
  }, [transport]);

  // Ctrl/Cmd+S saves (keymap `project.save`; also from text fields: the browser's own "save
  // page" is never wanted).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (matchesAction("project.save", e)) {
        e.preventDefault();
        saveRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="eth-project" data-feature="project">
      {/* Same size as the left rail's icon buttons, so it sits on the rail's column. */}
      <Popover
        aria-label={t("project.menu.label")}
        placement="bottom-start"
        trigger={(p) => (
          <IconButton
            size="lg"
            tone="ghost"
            className="eth-project__menu"
            label={t("project.menu.button")}
            icon={<Menu />}
            data-tour="project-menu"
            disabled={!transport}
            {...p}
          />
        )}
      >
        {(close) => <ProjectMenuPanel commands={commands} close={close} />}
      </Popover>
      <ProjectScreen commands={commands} />
      <ExportProjectDialog commands={commands} />
      {/* templates: save/insert/rename/delete template dialogs (track menu, palette, project screen). */}
      <TemplateDialogs />
      {renaming && name !== null && transport ? (
        <ProjectNameInput
          name={name}
          onDone={(next) => {
            setRenaming(false);
            const id = useProjectStore.getState().project?.id;
            if (next !== null && next.trim() && next.trim() !== name && id) void send(cmd("Project", { type: "Rename", id, name: next.trim() }));
          }}
        />
      ) : (
        <button
          type="button"
          className={name !== null ? "eth-project__name eth-user-text" : "eth-project__name"}
          data-testid="project-name"
          title={name ? t("project.name.rename_hint") : undefined}
          disabled={name === null}
          onClick={() => setRenaming(true)}
        >
          {name ?? t("project.name.none")}
        </button>
      )}
      {/* Always laid out (hidden when saved), so autosaving never shifts the top bar. */}
      {dirty ? (
        <span className="eth-project__dirty" role="status" aria-label={t("project.name.unsaved")} title={t("project.name.unsaved")}>
          ●
        </span>
      ) : (
        <span className="eth-project__dirty eth-project__dirty--clean" aria-hidden>
          ●
        </span>
      )}
      <SafeModeBanner />
      {error && !open && (
        <DismissibleError className="eth-project__error" onDismiss={clearError}>
          {error}
        </DismissibleError>
      )}
    </div>
  );
}

/** The project's name as a field: Enter or a click away renames, Escape cancels (`null`). */
function ProjectNameInput({ name, onDone }: { name: string; onDone(name: string | null): void }) {
  const t = useT();
  const done = useRef(false);
  const finish = (value: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(value);
  };
  return (
    <input
      className="eth-project__rename"
      data-testid="project-name-input"
      aria-label={t("project.name.input")}
      defaultValue={name}
      autoFocus
      spellCheck={false}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => finish(e.currentTarget.value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(e.currentTarget.value);
        else if (e.key === "Escape") finish(null);
      }}
    />
  );
}
