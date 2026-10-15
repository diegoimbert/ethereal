import { Ruler, type RulerProps } from "@/timeline";
import { RULER_TEMPO_EDITOR, RULER_TEMPO_MENU_ONLY } from "./rulerTempo";

/**
 * The timeline ruler with tempo-map editing (the arrangement's and the automation editor's).
 * `markers={false}`: its tempo and signature changes are shown elsewhere (`TempoMapMarkers`);
 * right-click still adds them.
 */
export function TempoRuler({ markers = true, ...props }: RulerProps & { markers?: boolean }) {
  return <Ruler {...props} tempoEditor={markers ? RULER_TEMPO_EDITOR : RULER_TEMPO_MENU_ONLY} />;
}
