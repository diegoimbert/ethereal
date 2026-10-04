// base-131: what happens on launch. Nothing is opened: hosts connect with no project, and
// the project screen is the first thing shown (plugins load only once a project is picked:
// a plugin that crashes the app can't take the next launch down with it).
//
// Settings > General "Reopen last project on launch" (default off) reopens the last
// project, but only when the previous session closed cleanly (`previousSession`: the
// engine's session markers natively, the page's own record in the browser). After a crash
// nothing is opened, whatever the setting: the crash dialog offers the project, with
// "Open without plugins".
import { useEffect } from "react";
import { create } from "zustand";
import type { ProjectId } from "@/generated";
import { LAST_PROJECT_KEY, previousSession, trackWebSession } from "@/features/versions/session";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { useProjectScreen } from "./screenStore";

/** localStorage: "Reopen last project on launch" (`"1"` = on). */
export const REOPEN_KEY = "eth.project.reopen-last";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Storage unavailable (private mode): the setting lasts for this session only.
  }
}

interface LaunchPrefs {
  reopenLast: boolean;
  setReopenLast(on: boolean): void;
}

/** The launch preference (a UI setting, per host). */
export const useLaunchPrefs = create<LaunchPrefs>((set) => ({
  reopenLast: read(REOPEN_KEY) === "1",
  setReopenLast: (on) => {
    write(REOPEN_KEY, on ? "1" : null);
    set({ reopenLast: on });
  },
}));

/** The project to reopen: the setting is on, the last session closed cleanly and the project still exists. */
export async function projectToReopen(transport: EngineTransport): Promise<ProjectId | null> {
  if (!useLaunchPrefs.getState().reopenLast) return null;
  const last = read(LAST_PROJECT_KEY);
  if (!last) return null;
  const [session, list] = await Promise.all([previousSession(transport), transport.send(cmd("Project", { type: "List" }))]);
  if (!session.clean) return null;
  if (list.type !== "Projects" || !list.projects.some((p) => p.id === last)) return null;
  return last;
}

/**
 * Connected with no project open: reopen the last one (see `projectToReopen`) or show the
 * project screen. Never throws: on any failure the project screen is shown.
 */
export async function launch(transport: EngineTransport): Promise<void> {
  try {
    const id = await projectToReopen(transport);
    if (id && !useProjectStore.getState().project) {
      await transport.send(cmd("Project", { type: "Open", id }));
      return;
    }
  } catch (e) {
    console.warn("Ethereal: reopening the last project failed", e);
  }
  if (!useProjectStore.getState().project) useProjectScreen.getState().show();
}

/**
 * Remembers the open project (what the setting reopens) and, in the browser, whether this
 * session ends cleanly (`trackWebSession`).
 */
export function useLaunchBookkeeping(transport: EngineTransport | null): void {
  const id = useProjectStore((s) => s.project?.id ?? null);
  useEffect(() => {
    if (id) write(LAST_PROJECT_KEY, id);
  }, [id]);
  // The local engine's session, also while a remote engine is in use.
  const local = useTransportSwitch()?.local ?? transport;
  useEffect(() => (local ? trackWebSession(local) : undefined), [local]);
}
