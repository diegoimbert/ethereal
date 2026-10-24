/**
 * Requests to show a library item in the browser from elsewhere (the quick search, ⌘F):
 * the indexed browser (`v2/BrowserV2.tsx`) takes the latest request when it mounts or when
 * a new one comes in. Samples and MIDI files are revealed in the folder tree (expanded down
 * to them, focused); presets and projects are searched by name.
 */
import { create } from "zustand";
import type { LibraryItem } from "@/generated";

export interface BrowserRevealRequest {
  item: Pick<LibraryItem, "id" | "kind" | "name" | "root" | "path">;
  /** Bumped by every request (the same item twice reveals it twice). */
  n: number;
}

interface BrowserRevealState {
  request: BrowserRevealRequest | null;
  reveal(item: BrowserRevealRequest["item"]): void;
}

export const useBrowserReveal = create<BrowserRevealState>((set) => ({
  request: null,
  reveal: (item) => set((s) => ({ request: { item, n: (s.request?.n ?? 0) + 1 } })),
}));

/** Show `item` in the browser (the caller opens the library panel). */
export const revealInBrowser = (item: BrowserRevealRequest["item"]): void => useBrowserReveal.getState().reveal(item);
