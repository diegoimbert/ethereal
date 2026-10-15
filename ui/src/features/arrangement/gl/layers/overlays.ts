/**
 * Phase-2c overlays of the WebGL arranger (new-track ghost lanes), mounted with the renderer
 * by `GlClipLayer`.
 */

import type { Row } from "../../layout";
import type { ArrangerRenderer } from "../renderer";
import { mountGhostLanes } from "./ghostLanes";

export interface OverlayLayers {
  /** The theme changed: re-read the tokens the layers draw with. */
  onTheme(): void;
  dispose(): void;
}

export function mountOverlayLayers(renderer: ArrangerRenderer, rows: () => ReadonlyArray<Row>): OverlayLayers {
  const ghosts = mountGhostLanes(renderer, rows);
  return {
    onTheme: () => {},
    dispose: () => {
      ghosts();
    },
  };
}
