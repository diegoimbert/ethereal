// Recents badges (base-114): "(local copy)" backups the engine keeps when a session replaced
// a differing stored copy (`collab_backup`: the name gets a " (local copy)" suffix), and the
// projects last used in a collaboration session. The engine has no such flag, so this device
// remembers it (localStorage, per device/browser profile; the Share redesign replaces it with
// engine-side sharing info).
import { useEffect, useSyncExternalStore } from "react";
import { useCollabStore } from "@/features/collab/store";
import { useProjectStore } from "@/state";

/** Suffix the engine gives the backup it keeps of a local version (collab/mod.rs). */
export const LOCAL_COPY_SUFFIX = " (local copy)";

/** `"Song (local copy)"` → `{ base: "Song", localCopy: true }`. */
export function splitLocalCopy(name: string): {
  base: string;
  localCopy: boolean;
} {
  return name.endsWith(LOCAL_COPY_SUFFIX) && name.length > LOCAL_COPY_SUFFIX.length
    ? { base: name.slice(0, -LOCAL_COPY_SUFFIX.length), localCopy: true }
    : { base: name, localCopy: false };
}

const KEY = "eth-collab-projects";

type Marks = Readonly<Record<string, string>>;

function load(): Marks {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as unknown;
    return v && typeof v === "object" ? (v as Marks) : {};
  } catch {
    return {};
  }
}

let marks: Marks = load();
const listeners = new Set<() => void>();

/** Remember that `projectId` was last used in `session`. */
export function markSessionProject(projectId: string, session: string): void {
  if (marks[projectId] === session) return;
  marks = { ...marks, [projectId]: session };
  try {
    localStorage.setItem(KEY, JSON.stringify(marks));
  } catch {
    // storage unavailable: this run only
  }
  for (const l of [...listeners]) l();
}

/** Test hook: forget everything (and re-read storage). */
export function resetSessionMarks(): void {
  marks = load();
  for (const l of [...listeners]) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

/** The session `projectId` was last used in, if any. */
export function useSessionOf(projectId: string): string | null {
  return useSyncExternalStore(subscribe, () => marks[projectId] ?? null);
}

/** Record the open project while online in a session (mounted by the project menu). */
export function useRecordSessionProjects(): void {
  const status = useCollabStore((s) => s.status);
  const projectId = useProjectStore((s) => s.project?.id ?? null);
  useEffect(() => {
    if (status.type === "Online" && projectId) markSessionProject(projectId, status.session);
  }, [status, projectId]);
}
