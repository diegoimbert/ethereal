// The arrangement timeline's menu, the same on the ruler's numbers and the marker band: tempo
// and signature changes, "add marker here", then the extensions' entries (leave a note). With
// the grid and snap helpers the marker band uses too.
import type { Beats } from "@/generated";
import { rulerMenuEntries } from "@/ext/surfaces";
import { RULER_TEMPO_MENU_ONLY } from "@/features/tempo/rulerTempo";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { t as tr } from "@/i18n";
import type { ContextMenuEntry } from "@/kit";
import { isReadOnly, useProjectStore } from "@/state";
import { resolveGrid, snapToGrid, TempoMap, type GridStep } from "@/timeline";
import { cmd, newId, type EngineTransport } from "@/transport";
import { sendEdit } from "./clipEditing";

/** The arrangement's resolved grid step at the current zoom (`null`: grid off). */
export function gridStep(tempo: TempoMap): GridStep | null {
  const s = arrangementView.getState();
  return resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
}

/** The same for the tempo-map chips (signature changes snap to it, or to a beat). */
export function tempoMapGridStep(): GridStep | null {
  const project = useProjectStore.getState().project;
  return project ? gridStep(TempoMap.fromProject(project)) : null;
}

/** Arrangement grid snap (alt/option bypasses it), as for clips. */
export function snap(beats: Beats, bypass: boolean): Beats {
  const project = useProjectStore.getState().project;
  if (bypass || !project) return Math.max(0, beats);
  const tempo = TempoMap.fromProject(project);
  return Math.max(0, snapToGrid(beats, gridStep(tempo), tempo));
}

export const addMarkerCommand = (position: Beats) => cmd("Marker", { type: "Add", id: newId(), position: Math.max(0, position), name: null, color: null });

/**
 * The timeline's menu after its tempo entries (the same on the ruler's numbers and the marker
 * band): "add marker here" (not for a viewer), then the extensions' (`arrangement.rulerMenu`,
 * e.g. leave a note). `beats` unsnapped; `bypassSnap`: alt/option held.
 */
export function timelineMenuExtras(transport: EngineTransport, beats: Beats, bypassSnap = false): ContextMenuEntry[] {
  const add: ContextMenuEntry[] = isReadOnly()
    ? []
    : [{ label: tr("clip_edit.marker.add_here"), onSelect: () => void sendEdit(transport, addMarkerCommand(snap(beats, bypassSnap))) }];
  return [...add, ...rulerMenuEntries(beats)];
}

/** The tempo entries and the extras ([`timelineMenuExtras`]), separated. */
export function timelineMenu(transport: EngineTransport, beats: Beats, bypassSnap = false): ContextMenuEntry[] {
  const project = useProjectStore.getState().project;
  const tempo =
    project && !isReadOnly()
      ? RULER_TEMPO_MENU_ONLY.menu(transport, TempoMap.fromProject(project), snap(beats, bypassSnap), {
          raw: beats,
          step: tempoMapGridStep(),
          free: bypassSnap,
        })
      : [];
  const extra = timelineMenuExtras(transport, beats, bypassSnap);
  return tempo.length && extra.length ? [...tempo, "separator", ...extra] : [...tempo, ...extra];
}
