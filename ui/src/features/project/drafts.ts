/**
 * Draft projects: the empty project you land in when you dismiss the
 * project picker, and a new project from the menu, exist in the store (the engine creates a
 * project's folder at once) but only count once you change something. Until then a draft is
 * left out of the project lists, and leaving it untouched (opening another project, the next
 * launch) deletes it, as if it had never been saved. Draft ids are remembered in
 * localStorage (per host), so a draft left at quit is cleaned up on the next launch.
 *
 * Untitled projects: a draft changed without being named (renamed) is kept, but stays
 * "untitled" until the user names it; leaving it asks whether to keep or discard it
 * (`untitledLeave.ts`). Also remembered in localStorage (per host). Projects made any other
 * way (named in a form, imported, copied, joined) are never untitled.
 */
import type { ProjectId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, newProjectId, type EngineTransport } from "@/transport";
import { dateName } from "./projectNames";

const KEY = "eth.project.drafts";
const UNTITLED_KEY = "eth.project.untitled";

function read(key = KEY): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(key) ?? "[]") as unknown;
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function write(ids: ReadonlySet<string>, key = KEY): void {
  try {
    if (ids.size) localStorage.setItem(key, JSON.stringify([...ids]));
    else localStorage.removeItem(key);
  } catch {
    // Storage unavailable: drafts are only tracked for this session.
  }
}

let drafts = read();
let untitled = read(UNTITLED_KEY);
/** The transport `bindDrafts` follows (what leaving an untitled project talks to). */
let bound: EngineTransport | null = null;

/** Changed but never named (see the module docs). */
export function isUntitled(id: ProjectId): boolean {
  return untitled.has(id);
}

/** The project was named, or discarded: no longer untitled. */
export function forgetUntitled(id: ProjectId): void {
  if (!untitled.has(id)) return;
  untitled = new Set(untitled);
  untitled.delete(id);
  write(untitled, UNTITLED_KEY);
}

function markUntitled(id: ProjectId): void {
  if (untitled.has(id)) return;
  untitled = new Set(untitled);
  untitled.add(id);
  write(untitled, UNTITLED_KEY);
}

/**
 * The open project the user chose to discard on leaving (`untitledLeave.ts`): deleted once
 * another one is open, which may come later than the action (an import, joining a session).
 * An edit to it first, or "keep", calls it off.
 */
let discarding: ProjectId | null = null;

/** Discard `id` once it is left (`null`: call a pending discard off). */
export function discardWhenLeft(id: ProjectId | null): void {
  discarding = id;
}

/** The transport the drafts follow (`bindDrafts`), if any. */
export function draftsTransport(): EngineTransport | null {
  return bound;
}

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
  bound = transport;
  const off = useProjectStore.subscribe((s, prev) => {
    const id = s.project?.id;
    // Changed, or named by the user (a rename is not always a document edit): kept.
    const renamed = id === prev.project?.id && s.project?.settings.name !== prev.project?.settings.name;
    if (id && drafts.has(id) && (s.dirty || s.history.can_undo || renamed)) {
      setDraft(id, false);
      // Changed without a name: untitled until named.
      if (!renamed) markUntitled(id);
    } else if (id && renamed) forgetUntitled(id);
    const left = prev.project?.id;
    if (left && left !== id && drafts.has(left)) {
      setDraft(left, false);
      void transport.send(cmd("Project", { type: "Delete", id: left })).catch(() => undefined);
    }
    if (discarding) {
      if (left === discarding && left !== id) {
        // Discarded, and now left: the engine deletes it (versions, recovery data).
        discarding = null;
        forgetUntitled(left);
        void transport.send(cmd("Project", { type: "Discard", id: left })).catch(() => undefined);
      } else if (id === discarding && s.revision !== prev.revision) discarding = null; // edited again: kept
    }
  });
  return () => {
    off();
    if (bound === transport) bound = null;
  };
}

/** Tests: forget the drafts. */
export function resetDrafts(): void {
  drafts = new Set();
  write(drafts);
  untitled = new Set();
  write(untitled, UNTITLED_KEY);
  discarding = null;
}
