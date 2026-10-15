import { create } from "zustand";

/** The Export project dialog: which project it exports, and its settings. */
interface ExportProjectState {
  /** The project being exported (`null` = closed). */
  target: { id: string; name: string } | null;
  /** Pack the media files (remembered for the session). */
  includeMedia: boolean;
  show(id: string, name: string): void;
  hide(): void;
  setIncludeMedia(on: boolean): void;
}

export const useExportProject = create<ExportProjectState>((set) => ({
  target: null,
  includeMedia: true,
  show: (id, name) => set({ target: { id, name } }),
  hide: () => set({ target: null }),
  setIncludeMedia: (includeMedia) => set({ includeMedia }),
}));
