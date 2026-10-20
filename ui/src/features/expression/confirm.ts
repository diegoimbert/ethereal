// The expression lanes ask before throwing points away (the shared confirmation dialog,
// mounted by the shell). `lane` is the lane's shown name (`./labels`).
import { useConfirm } from "@/kit";
import { t } from "@/i18n";

/** Clearing a lane throws its curve away: ask first (undo still brings it back). */
export function confirmClearLane(lane: string, points: number, run: () => void): void {
  useConfirm.getState().ask({
    title: t("expression.confirm.clear_title", { lane }),
    body: t("expression.confirm.clear_body", { count: points }),
    action: t("expression.confirm.clear"),
    danger: true,
    run,
  });
}

/** Removing a lane with points asks first; an empty one goes at once. */
export function confirmRemoveLane(lane: string, points: number, run: () => void): void {
  if (points === 0) return run();
  useConfirm.getState().ask({
    title: t("expression.confirm.remove_title", { lane }),
    body: t("expression.confirm.remove_body", { count: points }),
    action: t("expression.confirm.remove"),
    danger: true,
    run,
  });
}

/** Clearing per-note curves asks first. */
export function confirmClearNotes(lane: string, notes: number, run: () => void): void {
  useConfirm.getState().ask({
    title: t("expression.confirm.clear_title", { lane }),
    body: t("expression.confirm.clear_notes_body", { lane, count: notes }),
    action: t("expression.confirm.clear"),
    danger: true,
    run,
  });
}
