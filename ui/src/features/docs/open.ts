// Opening the user docs from the app (the project menu, the palette, the welcome screen, the
// report form). Light: the docs themselves are a separate chunk.
//
// - Desktop: in the app, over the editor (`DocsRoot`), so they work offline and the project
//   and any live session stay as they are.
// - Web (and the mobile UI): `/docs` in a new tab. The editor tab keeps its project, its
//   engine and its session; the docs tab never boots an engine or claims the project store.
import { create } from "zustand";
import { isTauri } from "@/transport";
import { docsHref } from "./route";

export interface DocsOverlayState {
  open: boolean;
  /** `<section>/<page>` ("" = the start page). */
  path: string;
  anchor: string;
  /** Bumped by every `openDocs`: the view starts again at `path`. */
  request: number;
}

export const useDocsOverlay = create<DocsOverlayState>()(() => ({ open: false, path: "", anchor: "", request: 0 }));

export function closeDocs(): void {
  useDocsOverlay.setState({ open: false });
}

/** Open the docs at `path` (`<section>/<page>`, "" for the start page) and `anchor`. */
export function openDocs(path = "", anchor = "", opts: { inApp?: boolean } = {}): void {
  if (opts.inApp ?? isTauri()) {
    useDocsOverlay.setState((s) => ({ open: true, path, anchor, request: s.request + 1 }));
    return;
  }
  window.open(docsHref(path, anchor), "_blank", "noopener");
}
