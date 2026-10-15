// Helpers of the editor's error boundaries (PanelBoundary.tsx, #590).
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";

/** How long "reload" waits for the save before reloading anyway (ms). */
const SAVE_WAIT_MS = 3000;

/** A lazy chunk that failed to load (after a deploy, or offline), as browsers word it. */
export function isChunkLoadError(error: unknown, depth = 0): boolean {
  const e = error as { name?: string; message?: string; cause?: unknown } | null;
  if (!e || depth > 3) return false;
  return (
    e.name === "ChunkLoadError" ||
    /dynamically imported module|importing a module script failed|error loading dynamically imported|loading (css )?chunk .* failed/i.test(e.message ?? "") ||
    // Loaders may wrap the import's error.
    isChunkLoadError(e.cause, depth + 1)
  );
}

/** Save the open project (best effort, bounded), then reload the page. */
export async function saveAndReload(transport: EngineTransport | null, reload: () => void = () => window.location.reload()): Promise<void> {
  if (transport && useProjectStore.getState().project) {
    const save = transport.send(cmd("Project", { type: "Save" })).catch(() => undefined);
    await Promise.race([save, new Promise((r) => setTimeout(r, SAVE_WAIT_MS))]);
  }
  reload();
}
