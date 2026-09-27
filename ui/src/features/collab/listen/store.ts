// Listen-on-peer UI state (docs/COLLAB.md §9), mirrored from `CollabEvent::{ListenStatus,
// IceServers}` plus the receiver's own errors.
import { create } from "zustand";
import type { IceServer, ListenerLink, ListenState, SiteId } from "@/generated";

export interface ListenStoreState {
  listening: ListenState;
  /** This site's listeners (host side; shown, not managed here). */
  listeners: ListenerLink[];
  iceServers: IceServer[];
  /** Last error to show (a refused `Listen`, a receiver failure). */
  error: string | null;
  /** The mapped position is before the host's record start (count-in). */
  countIn: boolean;
  reset(): void;
}

const INITIAL = {
  listening: { type: "Off" } as ListenState,
  listeners: [] as ListenerLink[],
  iceServers: [] as IceServer[],
  error: null as string | null,
  countIn: false,
};

export const useListenStore = create<ListenStoreState>()((set) => ({
  ...INITIAL,
  reset: () => set({ ...INITIAL }),
}));

/** The host this site listens to (connecting or listening), else `null`. */
export function activeHost(l: ListenState): SiteId | null {
  return l.type === "Connecting" || l.type === "Listening" ? l.host : null;
}
