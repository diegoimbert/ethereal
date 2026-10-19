// The read-only view: the app shell shows a document as in the editor, but nothing edits it.
// An extension that shows one (e.g. a shared project's viewer, docs/EXTENSIONS.md) sets it
// together with a read-only transport, which refuses every document command (the boundary);
// this flag lets the UI hide or disable what would edit, so nothing looks editable. Nothing in
// the app itself sets it.
import { create } from "zustand";
import type { TrackId } from "@/generated";

export interface ViewerInfo {
  /** The document's name (the band's title). */
  name: string;
  /** Why a track's Solo / Mute do nothing here, or `null` when they work (default: they do). */
  trackBlocked?(track: TrackId): string | null;
}

interface ViewerState {
  viewer: ViewerInfo | null;
  setViewer(viewer: ViewerInfo | null): void;
}

export const useViewerStore = create<ViewerState>()((set) => ({
  viewer: null,
  setViewer: (viewer) => set({ viewer }),
}));

/** The app shows a document read-only (outside React). */
export const isReadOnly = (): boolean => useViewerStore.getState().viewer !== null;

/** The app shows a document read-only. */
export const useReadOnly = (): boolean => useViewerStore((s) => s.viewer !== null);

/** The viewer's info, or `null` in the editor. */
export const useViewer = (): ViewerInfo | null => useViewerStore((s) => s.viewer);

/** Why a track's Solo / Mute do nothing in the viewer, or `null` when they work (or in the editor). */
export function listenBlockedReason(viewer: ViewerInfo | null, track: TrackId): string | null {
  return viewer?.trackBlocked?.(track) ?? null;
}
