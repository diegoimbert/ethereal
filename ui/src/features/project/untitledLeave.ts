/**
 * Leaving an untitled project (`drafts.ts`: changed, never named). It is already saved (the
 * project saves itself), so the question is not "save?" but "keep or discard?": keep (with a
 * name, if one is typed), discard (the project, its versions and its recovery data are
 * deleted), or cancel where leaving can be cancelled. A named project is kept without asking;
 * an untouched draft is still dropped silently (`drafts.ts`).
 *
 * Every in-app way out asks: opening, creating, importing a project (`guardLeave`), and the
 * desktop's quit (`features/quit`). Closing a browser tab doesn't: `beforeunload` can only show
 * the browser's generic "leave site?", which can't offer discard. The project stays saved there,
 * as after a crash, and the question comes the next time it is left in the app.
 */
import { create } from "zustand";
import type { Project, ProjectId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { discardWhenLeft, draftsTransport, forgetUntitled, isUntitled } from "./drafts";

export type UntitledChoice = { type: "cancel" } | { type: "discard" } | { type: "keep"; name: string };

/** Leaving the project for another one, or quitting the app (the buttons say which). */
export type UntitledLeaveKind = "leave" | "quit";

interface Pending {
  name: string;
  kind: UntitledLeaveKind;
  settle(choice: UntitledChoice): void;
}

export const useUntitledLeave = create<{ pending: Pending | null }>()(() => ({ pending: null }));

/** The open project, when leaving it asks (untitled, and the engine is there to act on it). */
export function untitledOpen(): Project | null {
  const project = useProjectStore.getState().project;
  return project && draftsTransport() && isUntitled(project.id) ? project : null;
}

/** Show the question (a newer one cancels an unanswered one). */
export function askUntitled(name: string, kind: UntitledLeaveKind): Promise<UntitledChoice> {
  useUntitledLeave.getState().pending?.settle({ type: "cancel" });
  return new Promise((resolve) => {
    const pending: Pending = {
      name,
      kind,
      settle: (choice) => {
        if (useUntitledLeave.getState().pending === pending) useUntitledLeave.setState({ pending: null });
        resolve(choice);
      },
    };
    useUntitledLeave.setState({ pending });
  });
}

/** The dialog's answer. */
export function answerUntitled(choice: UntitledChoice): void {
  useUntitledLeave.getState().pending?.settle(choice);
}

/** "Keep": named with `name` when one was typed; either way it is no longer asked about. */
export async function keepUntitled(t: EngineTransport, project: Project, name: string): Promise<void> {
  const trimmed = name.trim();
  discardWhenLeft(null);
  forgetUntitled(project.id);
  if (trimmed && trimmed !== project.settings.name) await t.send(cmd("Project", { type: "Rename", id: project.id, name: trimmed }));
}

/**
 * "Discard" on quitting: the engine deletes the open project (its folder: versions and
 * recovery data included) as it stops, and saves nothing of it meanwhile (`Project::Discard`).
 */
export async function discardUntitled(t: EngineTransport, id: ProjectId): Promise<void> {
  await t.send(cmd("Project", { type: "Discard", id }));
  forgetUntitled(id);
}

/**
 * Run `leave` (switching projects, or maybe not: a dismissed file picker), asking first when
 * the open project is untitled. "Discard" deletes it once another project is open
 * (`discardWhenLeft`: that may come after `leave` resolves, e.g. an import); if it stays
 * open, it stays as it is. Resolves `false` when cancelled.
 */
export async function guardUntitled(leave: () => Promise<boolean>): Promise<boolean> {
  const project = untitledOpen();
  const t = draftsTransport();
  if (!project || !t) return leave();
  const choice = await askUntitled(project.settings.name, "leave");
  if (choice.type === "cancel") return false;
  if (choice.type === "keep") await keepUntitled(t, project, choice.name).catch(() => undefined);
  else discardWhenLeft(project.id);
  const ran = await leave();
  // Cancelled by another guard (e.g. "leave the session?"): nothing is discarded.
  if (!ran && choice.type === "discard") discardWhenLeft(null);
  return ran;
}
