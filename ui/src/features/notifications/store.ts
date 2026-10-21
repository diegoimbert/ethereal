// User-facing notices as toasts: the engine's `Event::Notification` ("your local
// version was kept as …", "left the collaboration session …", export warnings) and the
// app's own short confirmations (project exported, imported, …).
import { create } from "zustand";
import type { Event, NotificationLevel } from "@/generated";
import { t, tText } from "@/i18n";
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
  /** A button on the toast (e.g. "resolve"). */
  action?: NoticeAction;
  /** When it was shown (ms since the epoch; the messages dialog). */
  at: number;
  /** How many times it came in a row while shown (the same level, title and text): one toast. */
  count: number;
}

export interface NoticeAction {
  label: string;
  onSelect: () => void;
}

export interface NotifyOptions {
  title?: string;
  onClick?: () => void;
  action?: NoticeAction;
}

/** Info toasts disappear after this long (paused while hovered). */
export const INFO_TOAST_MS = 6000;
/** Warning and error toasts disappear after this long (paused while hovered): longer, to read. */
export const ALERT_TOAST_MS = 10_000;
/** At most this many toasts show at once (an older one leaves the stack for the messages dialog). */
export const MAX_NOTICES = 3;
/** The messages dialog keeps this session's last this many. */
export const MAX_HISTORY = 100;

/** Title, accent colour and auto-dismiss delay of a toast for `level`. */
export function toastLook(level: NotificationLevel): {
  title: string;
  accent: string;
  timeoutMs: number | undefined;
} {
  switch (level) {
    case "Info":
      return {
        title: t("shell.notice.info"),
        accent: cssVar("info"),
        timeoutMs: INFO_TOAST_MS,
      };
    case "Warning":
      return { title: t("shell.notice.warning"), accent: cssVar("warn"), timeoutMs: ALERT_TOAST_MS };
    case "Error":
      return { title: t("shell.notice.error"), accent: cssVar("danger"), timeoutMs: ALERT_TOAST_MS };
  }
}

/** Engine messages are lowercase sentence fragments, shown as they are (the UI is lowercase). */
export function sentence(message: string): string {
  return message.trim();
}

interface NoticeState {
  /** Shown as toasts (the newest `MAX_NOTICES`). */
  notices: Notice[];
  /** This session's messages, oldest first (the messages dialog). */
  history: Notice[];
  /** Toasts pushed out of the stack before they were seen ("see all messages" shows then). */
  hidden: number;
  /** The messages dialog is open. */
  logOpen: boolean;
  push(level: NotificationLevel, message: string, opts?: NotifyOptions): number;
  dismiss(id: number): void;
  clear(): void;
  /** Open the messages dialog (the toasts go: they are all in it) or close it. */
  openLog(open: boolean): void;
  clearHistory(): void;
}

let nextId = 1;

export const useNotices = create<NoticeState>()((set) => ({
  notices: [],
  history: [],
  hidden: 0,
  logOpen: false,
  push: (level, message, opts) => {
    const id = nextId++;
    const notice: Notice = { id, level, message: sentence(message), ...opts, at: Date.now(), count: 1 };
    set((s) => {
      // The same message again while it shows: one toast counting them (its timer restarts).
      const same = (n: Notice) => n.level === notice.level && n.message === notice.message && n.title === notice.title;
      const shown = s.notices.find(same);
      if (shown) {
        const again = { ...notice, count: shown.count + 1 };
        const last = s.history.at(-1);
        return {
          notices: [...s.notices.filter((n) => n !== shown), again],
          history: last && same(last) ? [...s.history.slice(0, -1), again] : [...s.history, again].slice(-MAX_HISTORY),
        };
      }
      const all = [...s.notices, notice];
      return {
        notices: all.slice(-MAX_NOTICES),
        hidden: s.hidden + Math.max(0, all.length - MAX_NOTICES),
        history: [...s.history, notice].slice(-MAX_HISTORY),
      };
    });
    return id;
  },
  dismiss: (id) => set((s) => ({ notices: s.notices.filter((n) => n.id !== id) })),
  clear: () => set({ notices: [], hidden: 0, history: [], logOpen: false }),
  openLog: (open) => set(open ? { logOpen: true, notices: [], hidden: 0 } : { logOpen: false }),
  clearHistory: () => set({ history: [] }),
}));

/** Show a notice (also callable outside React). */
export function notify(level: NotificationLevel, message: string, opts?: NotifyOptions): number {
  return useNotices.getState().push(level, message, opts);
}

export type NotificationEvent = Extract<Event, { type: "Notification" }>;

/** Engine notifications a feature shows its own way (e.g. one summary instead of one per item). */
const filters = new Set<(event: NotificationEvent) => boolean>();

/** Drop the engine notifications `skip` returns true for (while registered); returns the unregister. */
export function skipNotifications(skip: (event: NotificationEvent) => boolean): () => void {
  filters.add(skip);
  return () => void filters.delete(skip);
}

/** Apply one engine event (ignores everything but `Notification`). */
export function onNotificationEvent(event: Event): void {
  if (event.type !== "Notification") return;
  for (const skip of filters) if (skip(event)) return;
  notify(event.level, tText(event.key, event.message));
}
