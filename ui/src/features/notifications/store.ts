// User-facing notices as toasts (base-114): the engine's `Event::Notification` ("your local
// version was kept as …", "left the collaboration session …", export warnings) and the
// app's own short confirmations (project exported, imported, …).
import { create } from "zustand";
import type { Event, NotificationLevel } from "@/generated";
import { cssVar } from "@/theme";

export interface Notice {
  id: number;
  level: NotificationLevel;
  /** Body text (the engine's message, first letter capitalized). */
  message: string;
  /** Optional title; defaults to the level's (`toastLook`). */
  title?: string;
  /** A click on the toast (e.g. open what it is about). */
  onClick?: () => void;
}

/** Info toasts disappear after this long (paused while hovered); warnings and errors stay. */
export const INFO_TOAST_MS = 6000;
/** At most this many notices are kept (the oldest go first). */
export const MAX_NOTICES = 3;

/** Title, accent colour and auto-dismiss delay of a toast for `level`. */
export function toastLook(level: NotificationLevel): {
  title: string;
  accent: string;
  timeoutMs: number | undefined;
} {
  switch (level) {
    case "Info":
      return {
        title: "Info",
        accent: cssVar("info"),
        timeoutMs: INFO_TOAST_MS,
      };
    case "Warning":
      return { title: "Warning", accent: cssVar("warn"), timeoutMs: undefined };
    case "Error":
      return { title: "Error", accent: cssVar("danger"), timeoutMs: undefined };
  }
}

/** Engine messages are lower-case sentence fragments: show them as sentences. */
export function sentence(message: string): string {
  const m = message.trim();
  return m ? m[0]!.toUpperCase() + m.slice(1) : m;
}

interface NoticeState {
  notices: Notice[];
  push(level: NotificationLevel, message: string, opts?: { title?: string; onClick?: () => void }): number;
  dismiss(id: number): void;
  clear(): void;
}

let nextId = 1;

export const useNotices = create<NoticeState>()((set) => ({
  notices: [],
  push: (level, message, opts) => {
    const id = nextId++;
    set((s) => ({
      notices: [...s.notices, { id, level, message: sentence(message), ...opts }].slice(-MAX_NOTICES),
    }));
    return id;
  },
  dismiss: (id) => set((s) => ({ notices: s.notices.filter((n) => n.id !== id) })),
  clear: () => set({ notices: [] }),
}));

/** Show a notice (also callable outside React). */
export function notify(level: NotificationLevel, message: string, opts?: { title?: string; onClick?: () => void }): number {
  return useNotices.getState().push(level, message, opts);
}

/** Apply one engine event (ignores everything but `Notification`). */
export function onNotificationEvent(event: Event): void {
  if (event.type === "Notification") notify(event.level, event.message);
}
