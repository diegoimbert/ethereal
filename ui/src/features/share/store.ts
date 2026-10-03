// Sharing UI state (docs/SHARING.md §8), mirrored from `Event::Share`: the `ShareState` the
// session pill and the Share popover show, and whether the popover is open.
import { create } from "zustand";
import type { Event, Participant, ShareRole, ShareState } from "@/generated";
import { pushNotice } from "./toasts";

export interface ShareUiState {
  state: ShareState;
  /** The Share popover (top bar) is open. */
  popoverOpen: boolean;
  setPopoverOpen(open: boolean): void;
  /** Apply one engine event (ignores everything but `Share` state and notices). */
  onEvent(event: Event): void;
  reset(): void;
}

const OFF: ShareState = { type: "Off" };

export const useShareStore = create<ShareUiState>()((set) => ({
  state: OFF,
  popoverOpen: false,
  setPopoverOpen: (popoverOpen) => set({ popoverOpen }),
  onEvent: (event) => {
    if (event.type !== "Share") return;
    const e = event.event;
    if (e.type === "State") set({ state: e.state });
    else if (e.type === "Notice") pushNotice(e.notice);
    // `PeerEndpoint` / `PeerSignal` belong to the web peer endpoint (`./endpoint`).
  },
  reset: () => set({ state: OFF, popoverOpen: false }),
}));

/** The participants of the current session (host first), or none. */
export function participantsOf(state: ShareState): Participant[] {
  return state.type === "Hosting" || state.type === "Joined" ? state.participants : [];
}

/** The host of a joined session (the first participant). */
export function hostOf(state: ShareState): Participant | null {
  if (state.type !== "Joined") return null;
  return state.participants.find((p) => p.role === "Host") ?? state.participants[0] ?? null;
}

/** This app joined with a listen link: the project is view only (docs/SHARING.md §8.4). */
export function useViewOnly(): boolean {
  return useShareStore((s) => s.state.type === "Joined" && s.state.role === "Listen");
}

/** The role this app joined with (`null` when not joined). */
export function useJoinedRole(): ShareRole | null {
  return useShareStore((s) => (s.state.type === "Joined" ? s.state.role : null));
}
