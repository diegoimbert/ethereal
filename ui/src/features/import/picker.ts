/**
 * "Import audio…" (⌘I / Ctrl+I, the command palette, the arrangement's and browser's menus):
 * the OS file dialog on the desktop (paths; the engine reads them), the browser's file
 * picker elsewhere (files; uploaded). The picked files land on the selected audio track at
 * the playhead, else on new audio tracks there.
 */
import { matchesAction } from "@/features/keymap";
import { playheadStore, useProjectStore, useSelectionStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { commandTarget, importAudio, type ImportOutcome } from "./importAudio";
import { isPathDropHost } from "./osDrop";
import { AUDIO_ACCEPT, fileSource, pathSource, type ImportSource } from "./sources";

/** The browser's file picker (multiple audio files). Resolves `[]` when dismissed. */
export function pickFiles(doc: Document = document): Promise<File[]> {
  return new Promise((resolve) => {
    const input = doc.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = AUDIO_ACCEPT;
    input.hidden = true;
    input.setAttribute("data-testid", "import-file-input");
    let done = false;
    const finish = (files: File[]) => {
      if (done) return;
      done = true;
      input.remove();
      resolve(files);
    };
    input.addEventListener("change", () => finish([...(input.files ?? [])]));
    input.addEventListener("cancel", () => finish([]));
    doc.body.appendChild(input);
    input.click();
  });
}

/** Ask the user for audio files (see the module docs). */
export async function pickSources(transport: EngineTransport): Promise<ImportSource[]> {
  if (isPathDropHost(transport)) {
    const paths = await transport.pickAudioFiles();
    return (paths ?? []).map(pathSource);
  }
  return (await pickFiles()).map(fileSource);
}

/** Pick files and import them (see the module docs). */
export async function openImportDialog(transport: EngineTransport): Promise<ImportOutcome[]> {
  if (!useProjectStore.getState().project) return [];
  const sources = await pickSources(transport);
  if (sources.length === 0) return [];
  const playhead = playheadStore.getPlayhead()?.transport.position ?? 0;
  return importAudio(transport, sources, commandTarget(useSelectionStore.getState().selectedTrack, playhead));
}

/** ⌘I / Ctrl+I (no other modifier). */
export function isImportShortcut(e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">): boolean {
  // keymap: `file.import` (default Mod+I).
  return matchesAction("file.import", e);
}
