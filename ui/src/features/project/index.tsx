// OWNERSHIP: the `ui-shell` node owns `ui/src/features/project/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ProjectMenu`: keep this export name and keep it prop-less (read state via hooks).
import "./project.css";
import { useEffect, useRef } from "react";
import { Menu } from "lucide-react";
import { useEngineCommands, useOptionalConnection } from "@/features/transport-bar/engine";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { launch, useLaunchBookkeeping } from "./launch";
import { ProjectScreen } from "./ProjectScreen";
import { SafeModeBanner } from "./SafeModeBanner";
import { useProjectScreen } from "./screenStore";

/**
 * Project menu: the Projects button, current project name and unsaved-changes dot. The
 * button opens the project screen (`ProjectScreen`: a modal over the app, also shown once on
 * launch: rename, new, open, duplicate, delete).
 *
 * The project saves itself: `AUTOSAVE_MS` after the last change (each edit restarts the
 * wait, so a burst of edits is one save). Ctrl/Cmd+S saves at once.
 *
 * Every operation goes through the engine-side project store (`Command::Project`); the UI
 * never touches files. The list and the dirty flag are mirrored in the project store from
 * `Event::Project`.
 */
/** Autosave delay after the last change (ms). */
export const AUTOSAVE_MS = 1000;

export function ProjectMenu() {
  const commands = useEngineCommands();
  const { transport, send, error, clearError } = commands;
  const name = useProjectStore((s) => s.project?.settings.name ?? null);
  const dirty = useProjectStore((s) => s.dirty);
  const open = useProjectScreen((s) => s.open);
  const disabled = !transport || name === null;

  const revision = useProjectStore((s) => s.revision);
  const save = () => void send(cmd("Project", { type: "Save" }));
  const saveRef = useRef(save);
  useEffect(() => {
    saveRef.current = disabled ? () => undefined : save;
  });

  // Launch (base-131): hosts open nothing, so once connected with no project, reopen the
  // last one if the setting allows it, else show the project screen. An engine that
  // already has a project open (a remote engine, the mock) shows the screen over it.
  const connected = useOptionalConnection()?.status === "connected";
  const launched = useRef<unknown>(null);
  useEffect(() => {
    if (!connected || !transport || launched.current === transport) return;
    launched.current = transport;
    if (useProjectStore.getState().project === null) {
      useProjectScreen.setState({ launchPending: false });
      void launch(transport);
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
    const t = setTimeout(() => saveRef.current(), AUTOSAVE_MS);
    return () => clearTimeout(t);
  }, [dirty, disabled, revision]);

  // Ctrl/Cmd+S saves (also from text fields: the browser's own "save page" is never wanted).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        saveRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="eth-project" data-feature="project">
      <Button tone="ghost" aria-label="Projects" title="Projects" disabled={!transport} onClick={() => useProjectScreen.getState().show()}>
        <Menu aria-hidden />
      </Button>
      <ProjectScreen commands={commands} />
      <span className="eth-project__name" data-testid="project-name" title={name ?? undefined}>
        {name ?? "No project"}
      </span>
      {/* Always laid out (hidden when saved), so autosaving never shifts the top bar. */}
      {dirty ? (
        <span className="eth-project__dirty" role="status" aria-label="Unsaved changes" title="Unsaved changes">
          ●
        </span>
      ) : (
        <span className="eth-project__dirty eth-project__dirty--clean" aria-hidden>
          ●
        </span>
      )}
      <SafeModeBanner />
      {error && !open && (
        <button type="button" className="eth-project__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}
    </div>
  );
}
