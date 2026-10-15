import type { RulerTempoEditor } from "@/timeline";
import { useProjectStore } from "@/state/projectStore";
import { rulerTempoMenu } from "./menus";
import { NoRulerMarkers, RulerTempoMarkers } from "./RulerTempoMarkers";
import { sortedSignatures, sortedTempoPoints } from "./tempoCommands";

/** The tempo-map editing of a timeline ruler: markers (drag), right-click to add a change. */
export const RULER_TEMPO_EDITOR: RulerTempoEditor = {
  Markers: RulerTempoMarkers,
  menu: (transport, tempo, beats, sig) => {
    const p = useProjectStore.getState().project;
    const points = sortedTempoPoints({ tempo_points: p?.tempo_points ?? {} });
    const signatures = sortedSignatures({ time_signatures: p?.time_signatures ?? {} });
    return rulerTempoMenu(transport, tempo, points, signatures, beats, sig);
  },
};

/** The same without markers (the tempo map is shown elsewhere: the marker lane); right-click still adds changes. */
export const RULER_TEMPO_MENU_ONLY: RulerTempoEditor = { ...RULER_TEMPO_EDITOR, Markers: NoRulerMarkers };
