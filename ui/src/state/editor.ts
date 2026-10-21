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
  /** Bumped by `dismiss`: the arrangement was clicked away from the edited clip. */
  dismissed: number;
  /** The latest `dismiss` closes the drawer (Escape) rather than collapsing it (a click away). */
  dismissCloses: boolean;
  /** Bumped by `revealClip`: the shell expands the editor drawer and collapses the inspector. */
  revealed: number;
  openClip(id: ClipId): void;
  /**
   * A double-click on a clip: `openClip`, and the shell expands the editor drawer and
   * collapses the inspector to its bar (unless pinned).
   */
  revealClip(id: ClipId): void;
  /**
   * Focus another clip of the open editor (multi-clip editing, a click on a clip's
   * band or note): changes `clip` without re-opening the editor.
   */
  focusClip(id: ClipId): void;
  close(): void;
  /**
   * A click away from the drawer (outside any clip): the shell collapses an unpinned drawer to
   * its bar (closes it when it has no clip to show). `close`: Escape, it closes.
   */
  dismiss(close?: boolean): void;
}

export const useEditorStore = create<EditorState>()((set) => ({
  clip: null,
  request: 0,
  dismissed: 0,
  dismissCloses: false,
  revealed: 0,
  openClip: (id) => set((s) => ({ clip: id, request: s.request + 1 })),
  revealClip: (id) => set((s) => ({ clip: id, request: s.request + 1, revealed: s.revealed + 1 })),
  focusClip: (id) => set((s) => (s.clip === id ? s : { clip: id })),
  close: () => set({ clip: null }),
  dismiss: (close = false) => set((s) => ({ dismissed: s.dismissed + 1, dismissCloses: close })),
}));

export const useEditedClipId = (): ClipId | null => useEditorStore((s) => s.clip);

/** Pointer travel (px) above which a press on a clip is a drag, not a click. */
export const CLICK_SLOP = 4;

/**
 * Runs `done(e)` on the next release of the pointer (`null` when it was cancelled). Used to
 * defer effects that change the layout (the drawer closing removes the scroll room under the
 * tracks, which could move rows under a held pointer, so the release and its click would land
 * on another row).
 */
function onNextRelease(done: (e: PointerEvent | null) => void): void {
  const off = () => {
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
  };
  const up = (e: PointerEvent) => (off(), done(e));
  const cancel = () => (off(), done(null));
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
}

/** A press away from the drawer: collapses an unpinned drawer on release. */
export function dismissOnRelease(): void {
  onNextRelease(() => useEditorStore.getState().dismiss());
}

/**
 * A press on a clip: runs `show` (shows the clip in the drawer) on release, if the pointer did
 * not travel (a click, not a drag-to-move/resize).
 */
export function showClipOnClick(from: { clientX: number; clientY: number }, show: () => void): void {
  onNextRelease((e) => {
    if (e && Math.hypot(e.clientX - from.clientX, e.clientY - from.clientY) <= CLICK_SLOP) show();
  });
}
