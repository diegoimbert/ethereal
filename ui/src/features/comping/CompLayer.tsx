/**
 * The comp on a track's main lane: what the comp regions play, drawn like clips (each piece
 * is its take clip trimmed to the region) but inert and outlined, so the main lane shows
 * what you hear. Clicking a piece selects its region (Up/Down then pick another take) and
 * opens the take lanes. While a take is auditioned, the comp is dimmed.
 */

import clsx from "clsx";
import { memo, useMemo } from "react";
import type { Beats, Clip, Track } from "@/generated";
import { useProjectStore } from "@/state";
import type { TempoMap, TimelineViewport } from "@/timeline";
import { ClipView } from "@/features/arrangement/ClipView";
import { beatsCss, widthCss } from "@/features/arrangement/laneGeometry";
import { compPieces, type CompPiece } from "./model";
import { useCompingUi } from "./store";
import "./comping.css";

export interface CompLayerProps {
  track: Track;
  vp: TimelineViewport;
  visible: { start: Beats; end: Beats };
  tempo: TempoMap;
}

/** A piece as a clip: its take clip reshaped to the piece (display only; same id, so its
 * notes and peaks draw). */
function pieceClip(clip: Clip, p: CompPiece): Clip {
  return { ...clip, start: p.start, length: p.end - p.start, offset: p.offset };
}

export const CompLayer = memo(function CompLayer({ track, vp, visible, tempo }: CompLayerProps) {
  const clips = useProjectStore((s) => s.project?.clips);
  const regions = useProjectStore((s) => s.project?.comp_regions);
  const lanes = useProjectStore((s) => s.project?.take_lanes);
  const auditioned = useCompingUi((s) => s.audition.get(track.id) ?? null);
  const selected = useCompingUi((s) => (s.selected?.track === track.id ? s.selected.region : null));
  const pieces = useMemo(() => {
    if (!clips || !regions) return [];
    const p = { clips, comp_regions: regions, tracks: { [track.id]: track } };
    return compPieces(p, track.id, (b) => tempo.bpmAt(b));
  }, [clips, regions, track, tempo]);
  if (pieces.length === 0 || !clips) return null;
  const color = track.color;
  return (
    <div className={clsx("eth-comp", auditioned && "eth-comp--auditioning")} data-comp={track.id}>
      {pieces.map((p) => {
        const clip = clips[p.clip];
        if (!clip || p.end < visible.start || p.start > visible.end) return null;
        const shaped = pieceClip({ ...clip, name: lanes?.[p.lane]?.name ?? clip.name }, p);
        return (
          <div
            key={`${p.region}:${p.index}`}
            className={clsx("eth-comp__piece", selected === p.region && "eth-comp__piece--selected")}
            data-comp-region={p.region}
            style={{ left: beatsCss(p.start - vp.scrollBeats), width: widthCss(p.end - p.start) }}
            title={`Comp: ${lanes?.[p.lane]?.name ?? "take"} (click to select, then Up/Down for another take)`}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              e.stopPropagation();
              useCompingUi.getState().select({ track: track.id, region: p.region });
              useCompingUi.getState().toggle(track.id, true);
            }}
          >
            <ClipView clip={shaped} bounds={{ track: track.id, start: p.start, length: p.end - p.start, offset: p.offset }} trackColor={color} vp={{ ...vp, scrollBeats: p.start }} visible={visible} tempo={tempo} ghost />
          </div>
        );
      })}
    </div>
  );
});
