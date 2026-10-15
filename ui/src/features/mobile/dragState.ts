/** A touch drag in progress (#522): where the finger is and what it does (the drag badge). */
import { create } from "zustand";
import type { ClipGrab } from "./clipTouch";

interface TouchDragState {
  /** The finger (client px), while a clip drag goes on. */
  point: { x: number; y: number } | null;
  grab: ClipGrab | null;
}

export const useTouchDrag = create<TouchDragState>()(() => ({ point: null, grab: null }));
