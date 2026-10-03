import type { Beats, Clip } from "@/generated";
import { songToContent } from "@/features/piano-roll/clipTime";
import type { EditorCursorMapping } from "./EditorPresence";

/**
 * Content x of a peer at song position `song` in `clip`, or `null` when the clip doesn't
 * play there. Pure (the per-frame part of `EditorPlayheads`).
 */
export function editorPlayheadX(clip: Clip | undefined, song: Beats, mapping: Pick<EditorCursorMapping, "toScreen">): number | null {
  if (!clip) return null;
  const content = songToContent(clip, song);
  return content === null ? null : mapping.toScreen(content, 0).x;
}
