// A render in progress (`TransportState.rendering`, CONTRACTS.md §13.19): an audio export,
// stems or a share snapshot renders while the live engine is paused. The transport plays the
// render's master as far as it is rendered (`start..until`); past it, nothing plays yet. Record
// waits until the render ends; edits apply at once and are heard once it ends.
import type { RenderingState } from "@/generated";
import { useProjectStore } from "@/state";
import { notify } from "@/features/notifications";
import { t as tr } from "@/i18n";

/** The render in progress, or `null`. */
export function useRendering(): RenderingState | null {
  return useProjectStore((s) => s.transport?.rendering ?? null);
}

/** {@link useRendering} outside React. */
export function rendering(): RenderingState | null {
  return useProjectStore.getState().transport?.rendering ?? null;
}

/** The job's progress in whole percent (0..100). */
export function renderPercent(r: RenderingState): number {
  return Math.round(Math.min(1, Math.max(0, r.progress)) * 100);
}

/** "rendering 42%". */
export function renderingText(r: RenderingState): string {
  return tr("shell.transport.rendering", { percent: renderPercent(r) });
}

/**
 * For record's own triggers (palette entries, their shortcuts): while a render runs, say so
 * and return `true` (the engine would refuse it with the same reason).
 */
export function refuseRecordWhileRendering(): boolean {
  if (!rendering()) return false;
  notify("Info", tr("error.render.running"));
  return true;
}
