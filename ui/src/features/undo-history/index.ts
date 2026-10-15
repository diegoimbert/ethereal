// The left rail mounts `HistoryPanel` (tab "history"); keep it prop-less.
import "./undo-history.css";
import { openLeftTab } from "@/state/shell";
import { useUndoHistoryUi } from "./store";

/** Undo history panel: the steps as a timeline, jump to a state, named checkpoints. */
export { HistoryPanel } from "./HistoryPanel";
export { useUndoHistoryUi } from "./store";

/** Open the History tab (palette, menus). */
export function openHistory(): void {
  openLeftTab("history");
}

/** Open the History tab and name the current step (palette "Name checkpoint…"). */
export function nameCurrentCheckpoint(): void {
  useUndoHistoryUi.getState().requestEditCurrent();
  openHistory();
}
