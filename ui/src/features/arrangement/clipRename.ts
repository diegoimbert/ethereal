// Renaming clips in place, in a clip's header in the arrangement (the clip menu's "Rename").
// With several clips selected, the field shows over the clicked one and its name goes to all.
import { create } from "zustand";
import type { ClipId, Command, Project } from "@/generated";
import { defaultClipName } from "@/state";
import { cmd } from "@/transport";
import { batchStep, countedStep } from "./editMath";

/** `clip`: the one whose header shows the field; `ids`: every clip the name goes to. */
export const useClipRename = create<{ clip: ClipId | null; ids: readonly ClipId[] }>()(() => ({ clip: null, ids: [] }));

/** Start renaming `clip` in its header, the name going to `ids` too (`null`: stop). */
export function renameClipInPlace(clip: ClipId | null, ids: readonly ClipId[] = clip ? [clip] : []): void {
  useClipRename.setState({ clip, ids: clip ? (ids.includes(clip) ? ids : [clip, ...ids]) : [] });
}

/**
 * The edit giving `name` to every clip of `ids` (an empty name: each its own default name),
 * leaving out the ones already named so: one `Clip::Rename`, or one undo step for several.
 * `null` when nothing changes.
 */
export function renameClipsCommand(project: Project, ids: readonly ClipId[], name: string): Command | null {
  const trimmed = name.trim();
  const commands: Command[] = [];
  for (const id of ids) {
    const clip = project.clips[id];
    if (!clip) continue;
    const next = trimmed || defaultClipName(project, clip);
    if (next !== clip.name) commands.push(cmd("Clip", { type: "Rename", id, name: next }));
  }
  if (commands.length <= 1) return commands[0] ?? null;
  return batchStep(countedStep("undo.clips.rename", commands.length, "rename clip", "rename clips"), commands);
}
