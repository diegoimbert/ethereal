// OWNERSHIP: the `ui-shell` node owns `ui/src/features/project/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ProjectMenu`: keep this export name and keep it prop-less (read state via hooks).
import "./project.css";
import { useEffect, useRef, useState } from "react";
import { Menu } from "lucide-react";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { ProjectManager } from "./ProjectManager";

/**
 * Project menu: current project name + unsaved-changes dot, and the project manager
 * popover (list, new, open, save as, duplicate, rename, delete).
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
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const disabled = !transport || name === null;

  const revision = useProjectStore((s) => s.revision);
  const [saving, setSaving] = useState(false);
  const save = () => {
    setSaving(true);
    void send(cmd("Project", { type: "Save" })).finally(() => setSaving(false));
  };
  const saveRef = useRef(save);
  useEffect(() => {
    saveRef.current = disabled ? () => undefined : save;
  });

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

  // Close the popover on outside click.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && e.target instanceof Node && !rootRef.current.contains(e.target)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className="eth-project" data-feature="project" ref={rootRef}>
      <Button
        tone="ghost"
        aria-label="Projects"
        aria-haspopup="dialog"
        aria-expanded={open}
        title="Projects"
        disabled={!transport}
        onClick={() => setOpen((o) => !o)}
      >
        <Menu aria-hidden />
      </Button>
      <span className="eth-project__name" data-testid="project-name" title={name ?? undefined}>
        {name ?? "No project"}
      </span>
      {dirty && (
        <span className="eth-project__dirty" role="status" aria-label="Unsaved changes" title="Unsaved changes">
          ●
        </span>
      )}
      {saving && (
        <span className="eth-project__saving" role="status">
          Saving…
        </span>
      )}
      {error && !open && (
        <button type="button" className="eth-project__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}
      {open && <ProjectManager commands={commands} onClose={() => setOpen(false)} />}
    </div>
  );
}
