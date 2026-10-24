import { create } from "zustand";
import type { ClipId, Project, TrackId } from "@/generated";
import { useArrangementUi } from "@/features/arrangement/state";
import { useProjectStore } from "@/state";
import { useInspectorCloses } from "@/state/shell";
import { itemSelection } from "@/timeline";

/** What the inspector shows: the selected clips, or the selected track. */
export type InspectorTarget = { kind: "clips"; ids: ClipId[] } | { kind: "track"; id: TrackId };

/** The selection's subject: the selected clips, else the focused track, else null. */
function selectionTarget(): InspectorTarget | null {
  const clips = itemSelection.getState().selected.clip;
  if (clips.size > 0) return { kind: "clips", ids: [...clips] };
  const track = useArrangementUi.getState().trackFocus;
  return track ? { kind: "track", id: track } : null;
}

/** `target` without what the project no longer has (null when nothing is left). */
function existing(target: InspectorTarget | null, project: Project | null): InspectorTarget | null {
  if (!target || !project) return null;
  if (target.kind === "track") return project.tracks[target.id] ? target : null;
  const ids = target.ids.filter((id) => project.clips[id]);
  if (ids.length === target.ids.length) return target;
  return ids.length > 0 ? { kind: "clips", ids } : null;
}

/**
 * The inspector's subject. It follows the selection, and stays on the last one when the
 * selection empties (a click elsewhere never closes the inspector): only an explicit close
 * (`closeInspector`: Esc; the × collapses it to its bar), the subject's removal or another
 * project ends it.
 */
const useHeld = create<{ target: InspectorTarget | null }>()(() => ({ target: null }));

function follow() {
  const next = existing(selectionTarget(), useProjectStore.getState().project);
  if (next) useHeld.setState({ target: next });
}
itemSelection.subscribe((s, prev) => {
  if (s.selected.clip !== prev.selected.clip) follow();
});
useArrangementUi.subscribe((s, prev) => {
  if (s.trackFocus !== prev.trackFocus) follow();
});
useProjectStore.subscribe((s, prev) => {
  if (s.project === prev.project) return;
  const held = useHeld.getState().target;
  const kept = s.project?.id === prev.project?.id ? existing(held, s.project) : null;
  if (kept !== held) useHeld.setState({ target: kept });
});
useInspectorCloses.subscribe(() => useHeld.setState({ target: null }));

/** The inspector's subject, or null when it is closed (the inspector hides). */
export function useInspectorTarget(): InspectorTarget | null {
  return useHeld((s) => s.target);
}
