/**
 * The mobile UI's sheets (#522, the design's §7): which one is open. One at a time; opening
 * another replaces it. `Sheet.tsx` draws them; the back button / gesture closes the open one.
 */
import { create } from "zustand";
import type { ClipId, TrackId } from "@/generated";

export type SheetId = { kind: "track"; track: TrackId } | { kind: "clip"; clip: ClipId };

interface SheetState {
  sheet: SheetId | null;
}

export const useMobileSheets = create<SheetState>()(() => ({ sheet: null }));

export const openSheet = (sheet: SheetId): void => useMobileSheets.setState({ sheet });
export const closeSheet = (): void => useMobileSheets.setState({ sheet: null });
