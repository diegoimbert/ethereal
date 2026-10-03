// Recents menu actions on shared projects (docs/SHARING.md §8.5). Sharing commands act on
// the open project, so an action on another project opens it first (opening a shared
// project resumes its room with the same links; opening a copy reconnects it, §7).
import type { ShareState } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { inviteLinkOf, sessionProject, useShareSession } from "./shareState";

/** How long to wait for the room to open before giving up on "Copy invite link". */
export const LINK_TIMEOUT_MS = 15_000;

/** Resolve with `pick(state)` once it is not `null` (rejects after `timeoutMs`). */
export function waitForShare<T>(pick: (s: ShareState) => T | null, timeoutMs = LINK_TIMEOUT_MS): Promise<T> {
  return new Promise((resolve, reject) => {
    const now = pick(useShareSession.getState().state);
    if (now !== null) return resolve(now);
    const timer = setTimeout(() => {
      unsubscribe();
      reject(new Error("Couldn't get a link from the sharing service. Try again from the Share button."));
    }, timeoutMs);
    const unsubscribe = useShareSession.subscribe((s) => {
      const v = pick(s.state);
      if (v === null) return;
      clearTimeout(timer);
      unsubscribe();
      resolve(v);
    });
  });
}

async function ensureOpen(t: EngineTransport, id: string): Promise<void> {
  if (useProjectStore.getState().project?.id !== id) await t.send(cmd("Project", { type: "Open", id }));
}

/** Open project `id` and make sure this app hosts it. */
async function ensureHosting(t: EngineTransport, id: string): Promise<void> {
  await ensureOpen(t, id);
  const s = useShareSession.getState().state;
  if (s.type !== "Hosting" || s.project !== id) await t.send(cmd("Share", { type: "Start" }));
}

/** Refresh the project list (the share badges come from the stores' `share.json`). */
async function refreshList(t: EngineTransport): Promise<void> {
  const reply = await t.send(cmd("Project", { type: "List" }));
  if (reply.type === "Projects") useProjectStore.getState().setProjects(reply.projects);
}

/** Host: copy the project's invite link (the edit link, else the listen link). */
export async function copyInviteLink(t: EngineTransport, id: string): Promise<string> {
  let link = inviteLinkOf(useShareSession.getState().state, id);
  if (link === null) {
    await ensureHosting(t, id);
    link = await waitForShare((s) => inviteLinkOf(s, id));
  }
  await navigator.clipboard.writeText(link);
  return link;
}

/** Host: stop sharing project `id` (links and member keys revoked; copies stay offline). */
export async function stopSharing(t: EngineTransport, id: string): Promise<void> {
  await ensureHosting(t, id);
  await t.send(cmd("Share", { type: "Stop" }));
  await refreshList(t);
}

/** Copy: open it and reconnect to the host (unless it already is connecting or joined). */
export async function reconnectCopy(t: EngineTransport, id: string): Promise<void> {
  await ensureOpen(t, id);
  const s = useShareSession.getState().state;
  if (s.type === "Joining" || sessionProject(s) === id) return;
  await t.send(cmd("Share", { type: "Reconnect", project: id }));
}

/** Copy: forget the invite and member key; it becomes an ordinary private project. */
export async function makePrivateCopy(t: EngineTransport, id: string): Promise<void> {
  await t.send(cmd("Share", { type: "Detach", project: id }));
  await refreshList(t);
}
