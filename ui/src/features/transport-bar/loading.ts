// The open project's load (`TransportState.loading`): the engine's audio is suspended and
// play, stop and record wait until it is ready. The project can be explored meanwhile.
import type { LoadProgress } from "@/generated";
import { useProjectStore } from "@/state";
import { notify } from "@/features/notifications";
import { t as tr } from "@/i18n";

/** The load in progress, or `null` once the project is ready. */
export function useProjectLoading(): LoadProgress | null {
  return useProjectStore((s) => s.transport?.loading ?? null);
}

/** {@link useProjectLoading} outside React. */
export function projectLoading(): LoadProgress | null {
  return useProjectStore.getState().transport?.loading ?? null;
}

/** Done/total of the whole load (plugins and media together), 0..1. */
export function loadFraction(p: LoadProgress): number {
  const total = p.plugins_total + p.media_total;
  return total === 0 ? 1 : (p.plugins_loaded + p.media_loaded) / total;
}

/** What the load is doing now: plugins first, then media, then the audio starting. */
export function loadingText(p: LoadProgress): string {
  if (p.plugins_loaded < p.plugins_total) return tr("shell.transport.loading_plugins", { done: p.plugins_loaded, total: p.plugins_total });
  if (p.media_loaded < p.media_total) return tr("shell.transport.loading_media", { done: p.media_loaded, total: p.media_total });
  return tr("shell.transport.loading_starting");
}

/**
 * For the transport's own triggers (palette entries, their shortcuts): while the project
 * loads, say so and return `true` (the engine would refuse it with the same reason).
 */
export function refuseWhileLoading(): boolean {
  if (!projectLoading()) return false;
  notify("Info", tr("error.project.loading"));
  return true;
}
