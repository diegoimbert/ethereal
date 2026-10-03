// Recents' view of the sharing session (docs/SHARING.md §8.5): which project is live, and
// the open project's invite links. Mirrors `ShareEvent::State`; the Share popover and the
// session pill (`features/share`) keep their own state.
import { useEffect } from "react";
import { create } from "zustand";
import type { ParticipantSummary, ProjectSummary, ShareState } from "@/generated";
import { useEngineEvent, useOptionalTransport } from "@/features/transport-bar/engine";
import { cmd } from "@/transport";

interface ShareSessionState {
  state: ShareState;
  set(state: ShareState): void;
}

export const useShareSession = create<ShareSessionState>((set) => ({
  state: { type: "Off" },
  set: (state) => set({ state }),
}));

/** Keep `useShareSession` current (mounted once, by the project menu). */
export function useShareSessionSync(): void {
  const transport = useOptionalTransport();
  useEngineEvent((e) => {
    if (e.type === "Share" && e.event.type === "State") useShareSession.getState().set(e.event.state);
  });
  useEffect(() => {
    useShareSession.getState().set({ type: "Off" });
    // The engine answers with `ShareEvent::State`. Older engines (or none) leave it `Off`.
    transport?.send(cmd("Share", { type: "Get" })).catch(() => undefined);
  }, [transport]);
}

/** The project the session is about, if any (hosting it, or joined to it). */
export function sessionProject(state: ShareState): string | null {
  return state.type === "Hosting" || state.type === "Joined" ? state.project : null;
}

/** "Live": `id` is the open project and its session is connected. */
export function isLive(state: ShareState, id: string): boolean {
  if (sessionProject(state) !== id) return false;
  if (state.type === "Hosting") return state.signal.type === "Online";
  return state.type === "Joined" && state.link.type === "Online";
}

/** The invite link to copy for project `id`, if it is hosted now (the edit link first). */
export function inviteLinkOf(state: ShareState, id: string): string | null {
  if (state.type !== "Hosting" || state.project !== id) return null;
  return state.edit_link ?? state.listen_link;
}

/** Recents badge of a shared project. */
export interface ShareBadge {
  label: string;
  tone: "accent" | "default";
  title: string;
}

export function shareBadge(p: ProjectSummary): ShareBadge | null {
  const share = p.share;
  if (!share) return null;
  if (!share.active) {
    return {
      label: "Sharing ended",
      tone: "default",
      title:
        share.role === "Host"
          ? "You stopped sharing this project"
          : `${share.host_name || "The host"} stopped sharing. This copy stays on this computer.`,
    };
  }
  if (share.role === "Host") return { label: "Shared", tone: "accent", title: "You share this project. Its links work while Ethereal is open." };
  const can = share.role === "Listen" ? "You can listen" : "You can edit";
  const synced = share.last_synced_ms === null ? "" : `. Last synced ${new Date(share.last_synced_ms).toLocaleString()}`;
  return { label: `From ${share.host_name}`, tone: "accent", title: `${share.host_name}'s project. ${can}${synced}` };
}

/** At most `max` avatars, and how many more there are ("+N"). */
export function avatarStack(participants: ReadonlyArray<ParticipantSummary>, max = 3): { shown: ParticipantSummary[]; more: number } {
  return { shown: participants.slice(0, max), more: Math.max(0, participants.length - max) };
}
