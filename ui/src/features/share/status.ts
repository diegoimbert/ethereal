// What the top-bar session pill says (docs/SHARING.md §8.1).
import type { Participant, ShareState } from "@/generated";
import { hostOf } from "./store";

export type PillTone = "live" | "busy" | "idle";

export interface SessionStatus {
  tone: PillTone;
  label: string;
  /** Longer explanation (tooltip and accessible description). */
  detail: string;
}

/**
 * What the session pill says (docs/SHARING.md §8.1), or `null` when there is no session (the
 * top bar shows the Share button).
 */
export function sessionStatus(state: ShareState): SessionStatus | null {
  switch (state.type) {
    case "Hosting":
      if (state.signal.type === "Online") return { tone: "live", label: "Live", detail: "Sharing: people with a link can join" };
      if (state.signal.type === "Connecting") return { tone: "busy", label: "Connecting…", detail: "Opening the shared project to invites" };
      return {
        tone: "busy",
        label: "Not joinable",
        detail: `Can't reach the sharing service (${state.signal.reason}). People already here stay connected.`,
      };
    case "Joining":
      if (state.stage.type === "Failed") return null;
      if (state.stage.type === "HostOffline") return { tone: "idle", label: "Host offline", detail: "Waiting for the host's Ethereal to open" };
      return { tone: "busy", label: "Connecting…", detail: "Joining a shared project" };
    case "Joined": {
      const host = hostOf(state)?.name || "The host";
      if (state.link.type === "Online") return { tone: "live", label: "Live", detail: `In ${host}'s shared project` };
      if (state.link.type === "Connecting") return { tone: "busy", label: "Reconnecting…", detail: `Reconnecting to ${host} (attempt ${state.link.attempt})` };
      return { tone: "idle", label: `${host} offline`, detail: `${host} is offline: you're working on an offline copy` };
    }
    case "Off":
      return null;
  }
}

/** The others online (for the pill's avatars): never this user. */
export function othersOnline(participants: ReadonlyArray<Participant>): Participant[] {
  return participants.filter((p) => p.online && !p.you);
}
