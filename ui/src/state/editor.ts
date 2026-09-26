// Which clip the detail editor shows: the piano roll for MIDI clips, the warp/clip view for
// audio clips. The arrangement calls `openClip` (e.g. on double-click); the app shell reacts to
// `request` by switching the detail tab. Consumers must tolerate an id that no longer exists.
import { create } from "zustand";
import type { ClipId } from "@/generated";

export interface EditorState {
  /** Clip open in the detail editor, or null. */
  clip: ClipId | null;
  /** Bumped on every `openClip`, so reopening the same clip still re-focuses its editor. */
  request: number;
  openClip(id: ClipId): void;
  close(): void;
}

export const useEditorStore = create<EditorState>()((set) => ({
  clip: null,
  request: 0,
  openClip: (id) => set((s) => ({ clip: id, request: s.request + 1 })),
  close: () => set({ clip: null }),
}));

export const useEditedClipId = (): ClipId | null => useEditorStore((s) => s.clip);
