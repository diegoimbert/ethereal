/**
 * Join-flow UI state: invites waiting to be sent to the engine (the web `/join/` route
 * queues one before the engine is up) and the "Join with a link…" dialog.
 */
import { create } from "zustand";
import { useProjectScreen } from "@/features/project/screenStore";

export interface QueuedInvite {
  link: string;
  /** Called once the engine has the invite (the web entry strips the key from the URL). */
  onDelivered?: () => void;
}

interface JoinStore {
  queue: QueuedInvite[];
  /** "Join with a link…" dialog. */
  pasteOpen: boolean;
  enqueue(invite: QueuedInvite): void;
  take(): QueuedInvite[];
  setPasteOpen(open: boolean): void;
}

export const useJoinStore = create<JoinStore>((set, get) => ({
  queue: [],
  pasteOpen: false,
  enqueue: (invite) => set((s) => ({ queue: [...s.queue, invite] })),
  take: () => {
    const q = get().queue;
    if (q.length) set({ queue: [] });
    return q;
  },
  setPasteOpen: (pasteOpen) => set({ pasteOpen }),
}));

/**
 * Open an invite link (any form `parseInvite` accepts): sent with `Share::OpenInvite` as
 * soon as the engine is connected, which shows the join screen.
 */
export function openInvite(link: string, onDelivered?: () => void): void {
  // Joining takes over from the project screen: no launch screen once the shared project
  // loads, and an open one (a deep link arriving at launch) closes for the join screen.
  useProjectScreen.setState({ launchPending: false, open: false, mode: "home" });
  useJoinStore.getState().enqueue({ link, onDelivered });
}

/** Show the "Join with a link…" dialog (Share popover, command palette). */
export function openJoinWithLink(): void {
  useJoinStore.getState().setPasteOpen(true);
}
