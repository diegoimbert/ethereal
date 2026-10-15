// The read-only viewer: the app shell shows the document as in the editor, but nothing edits
// it. A read-only transport refuses every document command (the boundary); this flag lets the
// UI hide or disable what would edit, so nothing looks editable. Never set in this build.
import { create } from "zustand";
import type { StaticPermissions, TrackId } from "@/generated";
import { t } from "@/i18n";

export interface ViewerInfo {
  /** The document's name (the band's title). */
  name: string;
  permissions: StaticPermissions;
  /** Tracks whose Solo / Mute work: the ones with a stem, and groups holding some. */
  listenable: ReadonlySet<TrackId>;
}

interface ViewerState {
  viewer: ViewerInfo | null;
  setViewer(viewer: ViewerInfo | null): void;
}

export const useViewerStore = create<ViewerState>()((set) => ({
  viewer: null,
  setViewer: (viewer) => set({ viewer }),
}));

/** The app shows a shared project read-only (outside React). */
export const isReadOnly = (): boolean => useViewerStore.getState().viewer !== null;

/** The app shows a shared project read-only. */
export const useReadOnly = (): boolean => useViewerStore((s) => s.viewer !== null);

/** The viewer's info, or `null` in the editor. */
export const useViewer = (): ViewerInfo | null => useViewerStore((s) => s.viewer);

/** Why a track's Solo / Mute do nothing in the viewer, or `null` when they work (or in the editor). */
export function listenBlockedReason(viewer: ViewerInfo | null, track: TrackId): string | null {
  if (!viewer || viewer.listenable.has(track)) return null;
  return viewer.permissions.allow_tracks ? t("share.viewer.track_no_audio") : t("share.viewer.mix_only");
}
