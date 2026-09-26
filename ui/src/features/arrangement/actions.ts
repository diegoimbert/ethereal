/** Edit actions on the selected clips, shared by the toolbar and keyboard shortcuts. */

import type { EngineTransport } from "@/transport";
import { newId } from "@/transport";
import { useProjectStore } from "@/state";
import { itemSelection, playheadBeats } from "@/timeline";
import { isArrangementClip } from "./clipTime";
import { selectedClips, sendEdit } from "./context";
import { deleteCommand, duplicateCommand, splitCommand, toggleLoopCommand } from "./editMath";

export type ClipAction = "split" | "duplicate" | "delete" | "loop" | "select-all" | "deselect";

export function runClipAction(transport: EngineTransport, action: ClipAction): Promise<void> {
  const clips = selectedClips();
  switch (action) {
    case "split":
      return sendEdit(transport, splitCommand(clips, playheadBeats(), newId));
    case "duplicate": {
      // Select the copies, so repeated duplicates keep extending the pattern.
      const ids: string[] = [];
      const command = duplicateCommand(clips, () => {
        const id = newId();
        ids.push(id);
        return id;
      });
      return sendEdit(transport, command).then(() => {
        const project = useProjectStore.getState().project;
        const created = ids.filter((id) => project?.clips[id]);
        if (created.length) itemSelection.getState().select("clip", created, "replace");
      });
    }
    case "delete":
      return sendEdit(transport, deleteCommand(clips.map((c) => c.id)));
    case "loop":
      return sendEdit(transport, toggleLoopCommand(clips));
    case "select-all": {
      const project = useProjectStore.getState().project;
      const all = project ? Object.values(project.clips).filter(isArrangementClip).map((c) => c.id) : [];
      itemSelection.getState().select("clip", all, "replace");
      return Promise.resolve();
    }
    case "deselect":
      itemSelection.getState().clear("clip");
      return Promise.resolve();
  }
}

/** The action for a key press in the arrangement, or null. */
export function actionForKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): ClipAction | null {
  const mod = e.metaKey || e.ctrlKey;
  const k = e.key.toLowerCase();
  if (!mod && !e.altKey && (e.key === "Delete" || e.key === "Backspace")) return "delete";
  if (!mod && e.key === "Escape") return "deselect";
  if (mod && !e.shiftKey && !e.altKey) {
    if (k === "e") return "split";
    if (k === "d") return "duplicate";
    if (k === "a") return "select-all";
  }
  if (mod && e.shiftKey && k === "l") return "loop";
  return null;
}
