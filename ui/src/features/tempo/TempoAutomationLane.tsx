/**
 * The project's tempo map as an automation-style lane: the master track's "Tempo" lane in
 * the arrangement (mounted by the automation feature's `TrackAutomationLanes`, on the
 * arrangement's view). `TempoLane` does the editing; this wires it to the document (sorted
 * points, tempo map, the master track).
 */

import { useMemo } from "react";
import { usePlayheadPosition, useProjectStore } from "@/state";
import { useTempoMap, type TimelineViewStore } from "@/timeline";
import { TempoLane } from "./TempoLane";
import { formatBpm, sortedTempoPoints } from "./tempoCommands";
import { useT } from "@/i18n";
import "./tempo.css";

export interface TempoAutomationLaneProps {
  /** Horizontal viewport shared with the clip lanes. */
  view: TimelineViewStore;
  /** Lane height (px). */
  height: number;
}

export function TempoAutomationLane({ view, height }: TempoAutomationLaneProps) {
  const tempoPoints = useProjectStore((s) => s.project?.tempo_points);
  const points = useMemo(() => (tempoPoints ? sortedTempoPoints({ tempo_points: tempoPoints }) : []), [tempoPoints]);
  const tempo = useTempoMap();
  // The lane's sections belong to the master track (the time selection's track).
  const master = useProjectStore((s) => Object.values(s.project?.tracks ?? {}).find((t) => t.kind === "Master")?.id ?? null);
  return <TempoLane view={view} tempo={tempo} points={points} height={height} track={master} />;
}

/** The tempo at the playhead, in BPM (re-renders when the playhead moves). */
export function PlayheadBpm({ className }: { className?: string }) {
  const t = useT();
  const tempo = useTempoMap();
  const at = usePlayheadPosition();
  return (
    <output className={className} aria-label={t("tempo.lane.playhead_bpm")} data-testid="playhead-bpm">
      {formatBpm(Math.round(tempo.bpmAt(at) * 100) / 100)} bpm
    </output>
  );
}
