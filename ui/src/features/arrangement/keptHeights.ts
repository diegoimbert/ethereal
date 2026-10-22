// The tracks' heights (the arrangement's vertical zoom) are view state, not part of the
// document: they are kept on this computer per project (localStorage `eth.arr.heights.<project
// id>`) and set again when the project opens.
import { useEffect } from "react";
import type { ProjectId, TrackId } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { clampTrackHeight, TRACK_HEIGHT } from "./layout";
import { useArrangementUi } from "./uiStore";

export interface KeptHeights {
  /** The height of the tracks never resized on their own. */
  default: number;
  /** The tracks resized on their own. */
  heights: Record<string, number>;
}

const PREFIX = "eth.arr.heights.";
export const keptHeightsKey = (project: ProjectId) => `${PREFIX}${project}`;
/** Saves wait this long after the last change (a resize drag changes them every frame). */
const SAVE_DELAY_MS = 250;

export function loadKeptHeights(project: ProjectId): KeptHeights | null {
  try {
    const raw = localStorage.getItem(keptHeightsKey(project));
    if (!raw) return null;
    const v = JSON.parse(raw) as { default?: unknown; heights?: unknown };
    const heights: Record<string, number> = {};
    if (v.heights && typeof v.heights === "object") {
      for (const [id, h] of Object.entries(v.heights)) if (typeof h === "number" && Number.isFinite(h)) heights[id] = clampTrackHeight(h);
    }
    const def = typeof v.default === "number" && Number.isFinite(v.default) ? clampTrackHeight(v.default) : TRACK_HEIGHT;
    return { default: def, heights };
  } catch {
    return null;
  }
}

function saveKeptHeights(project: ProjectId, tracks: Readonly<Record<string, unknown>>): void {
  const { defaultHeight, heights } = useArrangementUi.getState();
  // Tracks since deleted are left out.
  const kept = Object.fromEntries([...heights].filter(([id]) => id in tracks));
  try {
    if (defaultHeight === TRACK_HEIGHT && Object.keys(kept).length === 0) localStorage.removeItem(keptHeightsKey(project));
    else localStorage.setItem(keptHeightsKey(project), JSON.stringify({ default: defaultHeight, heights: kept } satisfies KeptHeights));
  } catch {
    /* not kept (private mode, quota) */
  }
}

/**
 * Keep the track heights per project: the open project's are set when it opens (the default
 * ones for a project never resized), and saved as they change. Returns the unsubscribe.
 */
export function bindTrackHeightsToProject(): () => void {
  let current: ProjectId | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    if (timer === undefined) return;
    clearTimeout(timer);
    timer = undefined;
    const project = useProjectStore.getState().project;
    if (current && project?.id === current) saveKeptHeights(current, project.tracks);
  };
  const open = (id: ProjectId | null) => {
    current = id;
    if (!id) return;
    const kept = loadKeptHeights(id);
    useArrangementUi.setState({
      defaultHeight: kept?.default ?? TRACK_HEIGHT,
      heights: new Map(Object.entries(kept?.heights ?? {}) as Array<[TrackId, number]>),
    });
  };
  open(useProjectStore.getState().project?.id ?? null);
  const offProject = useProjectStore.subscribe((s, prev) => {
    const id = s.project?.id ?? null;
    if (id === current) return;
    // The previous project's last change, saved before its heights are replaced.
    if (timer !== undefined && prev.project && prev.project.id === current) {
      clearTimeout(timer);
      timer = undefined;
      saveKeptHeights(current, prev.project.tracks);
    }
    open(id);
  });
  const offUi = useArrangementUi.subscribe((s, prev) => {
    if (!current || (s.heights === prev.heights && s.defaultHeight === prev.defaultHeight)) return;
    clearTimeout(timer);
    timer = setTimeout(flush, SAVE_DELAY_MS);
  });
  const onHide = () => flush();
  if (typeof window !== "undefined") window.addEventListener("pagehide", onHide);
  return () => {
    flush();
    offProject();
    offUi();
    if (typeof window !== "undefined") window.removeEventListener("pagehide", onHide);
  };
}

/** Mounts `bindTrackHeightsToProject` for the shell's life. */
export function useKeepTrackHeights(): void {
  useEffect(() => bindTrackHeightsToProject(), []);
}
