/**
 * Join-flow UI state: invites waiting to be sent to the engine (the web `/join/` route
 * queues one before the engine is up) and the "Join with a link…" dialog.
 */
import { create } from "zustand";

export interface QueuedInvite {
  link: string;
  /** Called once the engine has the invite (the web entry strips the key from the URL). */
  onDelivered?: () => void;
}

interface JoinStore {
  queue: QueuedInvite[];
  /** "Join with a link…" dialog. */
  pasteOpen: boolean;
  /** The join screen is showing (base-131: the launch project screen steps aside). */
  screenShown: boolean;
  enqueue(invite: QueuedInvite): void;
  take(): QueuedInvite[];
  setPasteOpen(open: boolean): void;
}

export const useJoinStore = create<JoinStore>((set, get) => ({
  queue: [],
  pasteOpen: false,
  screenShown: false,
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
  useJoinStore.getState().enqueue({ link, onDelivered });
}

/** Whether the join screen is showing. */
export const useJoinScreenShown = (): boolean => useJoinStore((s) => s.screenShown);

/** Show the "Join with a link…" dialog (Share popover, command palette). */
export function openJoinWithLink(): void {
  useJoinStore.getState().setPasteOpen(true);
}
