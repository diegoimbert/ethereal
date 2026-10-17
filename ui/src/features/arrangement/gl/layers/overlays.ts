/**
 * Phase-2c overlays of the WebGL arranger (new-track ghost lanes, and the extensions' layers:
 * `@/ext` "arrangement.glLayer"), mounted with the renderer by `GlClipLayer`.
 */

import { contributions } from "@/ext";
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
  const extra = contributions("arrangement.glLayer").map((mount) => mount(renderer));
  return {
    onTheme: () => {
      for (const x of extra) x.onTheme();
    },
    dispose: () => {
      ghosts();
      for (const x of extra) x.remove();
    },
  };
}
