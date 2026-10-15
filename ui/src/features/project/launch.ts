// base-131: what happens on launch. Hosts connect with no project; the app opens a draft
// project (drafts.ts: the starter project, no plugins) so the arranger is there, and shows the
// project screen over it (plugins load only once a stored project is picked: a plugin that
// crashes the app can't take the next launch down with it).
//
// Settings > General "Reopen last project on launch" (default off) reopens the last
// project, but only when the previous session closed cleanly (`previousSession`: the
// engine's session markers natively, the page's own record in the browser). After a crash
// nothing is opened, whatever the setting: the crash dialog offers the project, with
// "Open without plugins".
//
// The picker comes first (#646): in the browser, where the launch is known before the engine
// is up (`pickerExpected`), it shows as soon as the app paints and waits for the engine
// there (its actions are enabled once launch is done, `launching`); elsewhere it shows as
// soon as launch has decided, before the launch draft is made.
import { useEffect } from "react";
import { create } from "zustand";
import type { ProjectId } from "@/generated";
import { LAST_PROJECT_KEY, previousSession, trackWebSession, webSessionLeftOpen } from "@/features/versions/session";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { createDraftProject } from "./drafts";
import { MISSING_PROJECT_NOTICE, projectStored, syncProjectUrl, urlProjectId } from "./projectUrl";
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
 * Whether launch will show the picker, known before the engine is up: in the browser (the
 * crash record is the page's own, `webSessionLeftOpen`), with nothing open, no project in the
 * address bar, no crash to offer first and no last project to reopen. Natively the crash
 * markers are the engine's: the picker waits for launch.
 */
export function pickerExpected(transport: EngineTransport): boolean {
  if (transport.kind !== "wasm" || useProjectStore.getState().project) return false;
  if (urlProjectId(transport)) return false;
  if (useLaunchPrefs.getState().reopenLast && read(LAST_PROJECT_KEY)) return false;
  return !webSessionLeftOpen();
}

/**
 * Show the picker before the engine is up when launch is known to show it (`pickerExpected`).
 * Its actions wait for launch (`launching`). Returns whether it was shown.
 */
export function showPickerEarly(transport: EngineTransport): boolean {
  const screen = useProjectScreen.getState();
  if (!screen.launching || screen.launchShown || !pickerExpected(transport)) return false;
  useProjectScreen.setState({ open: true, mode: "home", launchShown: true });
  return true;
}

/**
 * Connected with no project open: reopen the last one (see `projectToReopen`) or show the
 * project screen. Never throws: on any failure the project screen is shown.
 */
export async function launch(transport: EngineTransport): Promise<void> {
  try {
    await launchInner(transport);
  } finally {
    // A later launch (another engine) shows the picker again.
    useProjectScreen.setState({ launching: false, launchShown: false });
  }
}

async function launchInner(transport: EngineTransport): Promise<void> {
  // The project in the address bar (a reload, a bookmark, Back into the app) goes first;
  // after a crash it waits like any other (the crash dialog offers it, `previousSession`).
  let missing = false;
  try {
    const fromUrl = urlProjectId(transport);
    if (fromUrl && !useProjectStore.getState().project) {
      if (!(await projectStored(transport, fromUrl))) missing = true;
      else if ((await previousSession(transport)).clean) {
        await transport.send(cmd("Project", { type: "Open", id: fromUrl }));
        hideEarlyPicker();
        return;
      }
    }
  } catch (e) {
    console.warn("Ethereal: opening the project in the address bar failed", e);
  }
  if (missing) {
    // Not stored in this browser: the picker says so, the URL becomes `/`.
    if (await launchPicker(transport)) useProjectScreen.setState({ notice: MISSING_PROJECT_NOTICE });
    syncProjectUrl({ replace: true });
    return;
  }
  try {
    const id = await projectToReopen(transport);
    if (id && !useProjectStore.getState().project) {
      await transport.send(cmd("Project", { type: "Open", id }));
      hideEarlyPicker();
      return;
    }
  } catch (e) {
    console.warn("Ethereal: reopening the last project failed", e);
  }
  await launchPicker(transport);
}

/**
 * The launch draft behind the project picker. Returns whether the picker was shown.
 */
async function launchPicker(transport: EngineTransport): Promise<boolean> {
  if (useProjectStore.getState().project) {
    hideEarlyPicker();
    return false;
  }
  const show = () => {
    // Unless it is up already, or was shown early and dismissed since.
    if (!useProjectScreen.getState().launchShown) useProjectScreen.setState({ open: true, mode: "home", launchShown: true });
  };
  // The picker first, then the draft behind it; after a crash the crash dialog's question
  // (asked meanwhile) goes first: the picker waits for the draft, as before.
  if (
    await previousSession(transport).then(
      (s) => s.clean,
      () => false,
    )
  )
    show();
  // The arranger is there from the start: a draft project (the starter one, built-in devices
  // only, so no plugin loads) opens behind the picker; it is discarded if left untouched.
  try {
    await createDraftProject(transport);
  } catch (e) {
    console.warn("Ethereal: creating the launch draft failed", e);
  }
  show();
  return true;
}

/** Launch opened something else after all: the picker it showed (still on its home) goes. */
function hideEarlyPicker(): void {
  const s = useProjectScreen.getState();
  if (s.launchShown && s.open && s.mode === "home") s.hide();
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
