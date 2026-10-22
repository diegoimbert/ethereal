// In-clip editing overlays of the WebGL arranger (phase 2a): fades and crossfades
// (`clipFades.ts`), title badges (`clipBadges.ts`). Mounted by `GlClipLayer` with its
// renderer. Clip gain and warp markers need no layer: the waveform pipeline scales peaks by
// the clip gain (full-scale marks included) and maps them through the warp markers, as the
// DOM did; arrangement clips show no warp marker overlay.
import { contributions } from "@/ext";
import { useMediaRefs } from "@/features/media-refs";
import type { EngineTransport } from "@/transport";
import { CLIP_RADIUS } from "../../laneBodyGeometry";
import type { ArrangerRenderer } from "../renderer";
import { useProjectStore } from "@/state";
import { clipBadgesLayer, titleBadgesWidth, extensionBadgesChanged } from "./clipBadges";
import { clipFadesLayer } from "./clipFades";
import { fadePipeline } from "./fadePipeline";

export { CLIP_FADES_LAYER, type ClipFadesLayer, type FadeDecor } from "./clipFades";
export { CLIP_BADGES_LAYER, type BadgeKind, type ClipBadgesLayer } from "./clipBadges";

/** Adds the clip-editing layers to `renderer`; returns their removal. */
export function mountClipEditLayers(renderer: ArrangerRenderer, transport: EngineTransport): () => void {
  // One `fade` program per renderer (pipelines belong to one GL context).
  const pipeline = fadePipeline(CLIP_RADIUS);
  const offs = [
    renderer.addLayer(clipBadgesLayer(pipeline)),
    renderer.addLayer(clipFadesLayer(renderer, transport, pipeline)),
    // The badges sit before the name: it is laid out after them.
    renderer.addTitleReserve({ id: "clip-badges", side: "left", width: (clip, atlas) => titleBadgesWidth(atlas, clip) }),
  ];
  // A sample going missing (or found again) adds or drops a badge: the names move after it.
  offs.push(
    useMediaRefs.subscribe((s, prev) => {
      if (s.missing !== prev.missing) renderer.invalidateTitles();
    }),
    // A MIDI clip's controller lanes changed: its badge may come, go or change.
    useProjectStore.subscribe((s, prev) => {
      if (s.project?.expression_lanes !== prev.project?.expression_lanes) renderer.invalidateTitles();
    }),
    // An extension's badges changed (`@/ext` "clips.badge").
    ...contributions("clips.badge").map((p) =>
      p.subscribe(() => {
        extensionBadgesChanged();
        renderer.invalidateTitles();
      }),
    ),
  );
  return () => offs.forEach((off) => off());
}
