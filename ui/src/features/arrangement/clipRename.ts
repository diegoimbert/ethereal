// Renaming a clip in place, in its header in the arrangement (the clip menu's "Rename").
import { create } from "zustand";
import type { ClipId } from "@/generated";

export const useClipRename = create<{ clip: ClipId | null }>()(() => ({ clip: null }));

/** Start renaming `clip` in its header (`null`: stop). */
export function renameClipInPlace(clip: ClipId | null): void {
  useClipRename.setState({ clip });
}
