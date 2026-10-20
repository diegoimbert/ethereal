// Disk-streaming dropouts: long media stream from disk (native) or OPFS (web); when
// the reader can't keep up the engine plays silence and the controller reports
// `MediaEvent::StreamUnderruns` (throttled to about one per second). The transport bar shows
// a warning light next to the CPU meter while there are dropouts (click to reset), and the
// first dropout of a playback session raises one toast.
import { create } from "zustand";
import type { Event } from "@/generated";
import { notify, useNotices } from "@/features/notifications";
import { intlLocale, t } from "@/i18n";

interface DiskState {
  /** Dropouts since `since` (reset by the user). */
  dropouts: number;
  /** When the first dropout since the last reset happened (ms since epoch). */
  since: number | null;
  /** The toast of the current playback session, once raised. */
  toast: number | null;
  /** `true` once this playback session raised its toast. */
  toasted: boolean;
  onEvent(event: Event, now?: number): void;
  /** A new playback session (play started): its first dropout may toast again. */
  playbackStarted(): void;
  /** Clear the counter (the light goes away). */
  reset(): void;
}

export const useDiskStore = create<DiskState>()((set, get) => ({
  dropouts: 0,
  since: null,
  toast: null,
  toasted: false,
  onEvent: (event, now = Date.now()) => {
    if (event.type !== "Media" || event.event.type !== "StreamUnderruns") return;
    const count = event.event.count;
    if (count <= 0) return;
    const s = get();
    let { toast, toasted } = s;
    // One toast per playback session, and never two on screen.
    const showing = toast !== null && useNotices.getState().notices.some((n) => n.id === toast);
    if (!toasted && !showing) {
      toast = notify("Warning", t("shell.transport.disk_toast"), { title: t("shell.transport.disk_toast_title") });
      toasted = true;
    }
    set({ dropouts: s.dropouts + count, since: s.since ?? now, toast, toasted });
  },
  playbackStarted: () => set({ toasted: false }),
  reset: () => set({ dropouts: 0, since: null }),
}));

/** Tooltip of the disk light. */
export function diskTitle(dropouts: number, since: number | null): string {
  if (since === null) return t("shell.transport.disk_title", { count: dropouts });
  return t("shell.transport.disk_title_since", { count: dropouts, time: new Date(since).toLocaleTimeString(intlLocale()) });
}

export function resetDiskStore(): void {
  useDiskStore.setState({ dropouts: 0, since: null, toast: null, toasted: false });
}
