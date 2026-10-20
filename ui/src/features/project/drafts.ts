/**
 * Draft projects: the empty project you land in when you dismiss the
 * project picker, and a new project from the menu, exist in the store (the engine creates a
 * project's folder at once) but only count once you change something. Until then a draft is
 * left out of the project lists, and leaving it untouched (opening another project, the next
 * launch) deletes it, as if it had never been saved. Draft ids are remembered in
 * localStorage (per host), so a draft left at quit is cleaned up on the next launch.
 */
import type { ProjectId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, newProjectId, type EngineTransport } from "@/transport";
import { dateName } from "./projectNames";

const KEY = "eth.project.drafts";

function read(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]") as unknown;
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function write(ids: ReadonlySet<string>): void {
  try {
    if (ids.size) localStorage.setItem(KEY, JSON.stringify([...ids]));
    else localStorage.removeItem(KEY);
  } catch {
    // Storage unavailable: drafts are only tracked for this session.
  }
}

let drafts = read();

export function isDraft(id: ProjectId): boolean {
  return drafts.has(id);
}

function setDraft(id: ProjectId, draft: boolean): void {
  if (drafts.has(id) === draft) return;
  drafts = new Set(drafts);
  if (draft) drafts.add(id);
  else drafts.delete(id);
  write(drafts);
}

/** Set by test harnesses (e2e) whose tests start from an empty project. */
const EMPTY_PROJECTS_KEY = "eth.project.empty";

function emptyProjects(): boolean {
  try {
    return localStorage.getItem(EMPTY_PROJECTS_KEY) === "1";
  } catch {
    return false;
  }
}

/** Create (and open) a new draft project named after today's date. */
export async function createDraftProject(transport: EngineTransport): Promise<void> {
  const id = newProjectId();
  const name = dateName(useProjectStore.getState().projects);
  setDraft(id, true);
  try {
    // No template given: the default project template, else the starter project (a MIDI
    // track "Synth" with the built-in Synth). Test harnesses can ask for an empty one.
    if (emptyProjects()) await transport.send(cmd("Project", { type: "Create", id, name }));
    else await transport.send(cmd("Template", { type: "NewProject", id, name, template: null }));
  } catch (e) {
    setDraft(id, false);
    throw e;
  }
}

/**
 * Delete the drafts that are not open (left untouched); returns once done. `keep`: the
 * project about to open (the one in the address bar on a reload).
 */
export async function deleteStaleDrafts(transport: EngineTransport, keep: string | null = null): Promise<void> {
  // Read again: the tab that owned the store before this one may have kept drafts
  // since this page loaded the list.
  drafts = read();
  const open = useProjectStore.getState().project?.id;
  for (const id of [...drafts]) {
    if (id === open || id === keep) continue;
    await transport.send(cmd("Project", { type: "Delete", id })).catch(() => undefined);
    setDraft(id, false);
  }
}

/**
 * Follow the open project: a draft that gets changed (unsaved edits, or an undo step) is a
 * real project from then on; a draft switched away from untouched is deleted. Returns the
 * unsubscribe.
 */
export function bindDrafts(transport: EngineTransport): () => void {
  return useProjectStore.subscribe((s, prev) => {
    const id = s.project?.id;
    // Changed, or named by the user (a rename is not always a document edit): kept.
    const renamed = id === prev.project?.id && s.project?.settings.name !== prev.project?.settings.name;
    if (id && drafts.has(id) && (s.dirty || s.history.can_undo || renamed)) setDraft(id, false);
    const left = prev.project?.id;
    if (left && left !== id && drafts.has(left)) {
      setDraft(left, false);
      void transport.send(cmd("Project", { type: "Delete", id: left })).catch(() => undefined);
    }
  });
}

/** Tests: forget the drafts. */
export function resetDrafts(): void {
  drafts = new Set();
  write(drafts);
}
