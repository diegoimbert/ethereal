// Sharing toasts (docs/SHARING.md §8.7): `ShareEvent::Notice`s and the popover's own
// confirmations ("Link copied"), at most `MAX_SHARE_TOASTS` on screen.
import { create } from "zustand";
import type { ShareNotice } from "@/generated";
import { useProjectStore } from "@/state";
import { peerColor } from "@/features/collab/store";

export interface ShareToast {
  id: number;
  title: string;
  text?: string;
  /** CSS colour (a peer's colour for "Ada joined"); default: the accent token. */
  accent?: string;
}

/** At most this many on screen (the oldest go first), like the chat toasts. */
export const MAX_SHARE_TOASTS = 3;
/** Auto-dismiss delay (paused while hovered). */
export const SHARE_TOAST_MS = 6000;

interface ToastState {
  toasts: ShareToast[];
  push(t: Omit<ShareToast, "id">): number;
  dismiss(id: number): void;
  clear(): void;
}

let nextId = 1;

export const useShareToasts = create<ToastState>()((set) => ({
  toasts: [],
  push: (t) => {
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts, { id, ...t }].slice(-MAX_SHARE_TOASTS) }));
    return id;
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  clear: () => set({ toasts: [] }),
}));

/** Show a sharing toast (also callable outside React). */
export function shareToast(title: string, text?: string, accent?: string): number {
  return useShareToasts.getState().push({ title, text, accent });
}

const projectName = () => useProjectStore.getState().project?.settings.name || "the project";

/** The toast for an engine notice (§8.7 wording). */
export function noticeToast(notice: ShareNotice): Omit<ShareToast, "id"> {
  switch (notice.type) {
    case "ParticipantJoined":
      return { title: `${notice.name || "Someone"} joined`, accent: peerColor(notice.color) };
    case "ParticipantLeft":
      return { title: `${notice.name || "Someone"} left` };
    case "HostOffline":
      return { title: `${notice.host_name} went offline`, text: "You're on an offline copy." };
    case "HostBack":
      return { title: `${notice.host_name} is back` };
    case "SharingEnded":
      return { title: `${notice.host_name} stopped sharing ${projectName()}`, text: "Your copy stays on this computer." };
    case "LocalCopyKept":
      return { title: "Offline changes kept", text: `Saved as “${notice.name}”.` };
  }
}

export function pushNotice(notice: ShareNotice): void {
  useShareToasts.getState().push(noticeToast(notice));
}
