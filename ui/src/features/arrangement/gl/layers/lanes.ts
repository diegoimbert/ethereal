/**
 * The lane layers of the WebGL arranger (phase 2b): automation lanes, comping (the comp and
 * the take lanes) and group summaries. `mountLaneLayers` adds them to a renderer.
 */

import { useCompingUi } from "@/features/comping/store";
import type { ArrangerRenderer } from "../renderer";
import { automationLayer } from "./automationLanes";
import { compingLayers } from "./comping";
import { groupSummaryLayer } from "./groupSummary";
import { shapePipeline } from "./shapes";

/** Adds the lane layers to `r` (and redraws when their UI state changes). Returns their removal. */
export function mountLaneLayers(r: ArrangerRenderer): () => void {
  const shape = shapePipeline();
  const comping = compingLayers(shape);
  const offs = [
    r.addLayer(groupSummaryLayer()),
    r.addLayer(comping.under),
    r.addLayer(automationLayer(shape)),
    r.addLayer(comping.over),
    useCompingUi.subscribe(() => r.requestRender()),
  ];
  return () => offs.forEach((off) => off());
}
