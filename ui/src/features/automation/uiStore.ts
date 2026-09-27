/**
 * Automation UI state (not in the document): which tracks have their automation area open
 * and which parameters' lanes are shown, in order. Showing a lane does not create it: the
 * document lane is created with the first breakpoint. Module-level so it survives view
 * switches.
 *
 * Heights are a pure function of this state, so the arrangement can lay out its rows
 * without measuring the DOM (`automationHeight`, `useAutomationHeight`).
 */

import { useCallback } from "react";
import { create } from "zustand";
import type { TrackId } from "@/generated";
import type { ValueRange } from "./valueAxis";

/** Height of the per-track bar with the automation toggle and "show parameter" menu. */
export const AUTOMATION_BAR_HEIGHT = 20;
/** Default height of one automation lane. */
export const LANE_HEIGHT = 64;
/** Resized lanes stay within `[MIN_LANE_HEIGHT, MAX_LANE_HEIGHT]` (like tracks). */
export const MIN_LANE_HEIGHT = 32;
export const MAX_LANE_HEIGHT = 320;

export function clampLaneHeight(h: number): number {
  return Math.min(MAX_LANE_HEIGHT, Math.max(MIN_LANE_HEIGHT, Math.round(h)));
}

/** Key of one shown lane in per-lane UI state (`laneHeights`, `ranges`). */
export function laneUiKey(track: TrackId, key: string): string {
  return `${track}|${key}`;
}

export interface AutomationUiState {
  /** Tracks whose automation lanes are shown. */
  open: ReadonlySet<TrackId>;
  /** Target keys (`targetKey`) of the lanes shown per track, top to bottom. */
  shown: Readonly<Record<TrackId, ReadonlyArray<string>>>;
  /** Resized lane heights by `laneUiKey` (absent: `LANE_HEIGHT`). */
  laneHeights: Readonly<Record<string, number>>;
  /** Visible value window by `laneUiKey` (absent: the param's default window). */
  ranges: Readonly<Record<string, ValueRange>>;

  /** Open/close a track's lanes. `initial` seeds the shown list the first time. */
  setOpen(track: TrackId, open: boolean, initial?: ReadonlyArray<string>): void;
  /** Show a lane (appended; opens the track). */
  show(track: TrackId, key: string): void;
  /** Hide a lane (the document lane and its points stay). */
  hide(track: TrackId, key: string): void;
  /** Replace the lane at `from` with `to` (the lane header's parameter chooser). */
  replace(track: TrackId, from: string, to: string): void;
  /** Resize a lane (`null`: back to `LANE_HEIGHT`). */
  setLaneHeight(track: TrackId, key: string, height: number | null): void;
  /** Set a lane's visible value window (`null`: the default). */
  setRange(track: TrackId, key: string, range: ValueRange | null): void;
}

const INITIAL = { open: new Set<TrackId>() as ReadonlySet<TrackId>, shown: {}, laneHeights: {}, ranges: {} };

function without<T>(rec: Readonly<Record<string, T>>, key: string): Record<string, T> {
  const next = { ...rec };
  delete next[key];
  return next;
}

export const useAutomationUi = create<AutomationUiState>()((set) => ({
  ...INITIAL,
  setOpen: (track, open, initial) =>
    set((s) => {
      if (s.open.has(track) === open) return s;
      const next = new Set(s.open);
      if (open) next.add(track);
      else next.delete(track);
      const shown = open && !s.shown[track] && initial ? { ...s.shown, [track]: [...initial] } : s.shown;
      return { open: next, shown };
    }),
  show: (track, key) =>
    set((s) => {
      const cur = s.shown[track] ?? [];
      const open = s.open.has(track) ? s.open : new Set(s.open).add(track);
      if (cur.includes(key)) return { open };
      return { open, shown: { ...s.shown, [track]: [...cur, key] } };
    }),
  hide: (track, key) =>
    set((s) => {
      const cur = s.shown[track] ?? [];
      if (!cur.includes(key)) return s;
      return { shown: { ...s.shown, [track]: cur.filter((k) => k !== key) } };
    }),
  replace: (track, from, to) =>
    set((s) => {
      const cur = s.shown[track] ?? [];
      if (from === to) return s;
      const next = cur.filter((k) => k !== to).map((k) => (k === from ? to : k));
      return { shown: { ...s.shown, [track]: next } };
    }),
  setLaneHeight: (track, key, height) =>
    set((s) => {
      const k = laneUiKey(track, key);
      if (height === null) return k in s.laneHeights ? { laneHeights: without(s.laneHeights, k) } : s;
      const h = clampLaneHeight(height);
      return s.laneHeights[k] === h ? s : { laneHeights: { ...s.laneHeights, [k]: h } };
    }),
  setRange: (track, key, range) =>
    set((s) => {
      const k = laneUiKey(track, key);
      if (range === null) return k in s.ranges ? { ranges: without(s.ranges, k) } : s;
      return { ranges: { ...s.ranges, [k]: range } };
    }),
}));

/** Reset (tests). */
export function resetAutomationUi(): void {
  useAutomationUi.setState({ open: new Set(), shown: {}, laneHeights: {}, ranges: {} });
}

/** Keys of the lanes shown for `track` (empty when closed). */
export function shownKeys(state: Pick<AutomationUiState, "open" | "shown">, track: TrackId): ReadonlyArray<string> {
  return state.open.has(track) ? (state.shown[track] ?? EMPTY) : EMPTY;
}

const EMPTY: ReadonlyArray<string> = [];

/** The UI state heights depend on (`laneHeights` optional: default heights). */
export type HeightState = Pick<AutomationUiState, "open" | "shown"> & Partial<Pick<AutomationUiState, "laneHeights">>;

/** Height of one shown lane (resized, or `LANE_HEIGHT`). */
export function laneHeightOf(state: Partial<Pick<AutomationUiState, "laneHeights">>, track: TrackId, key: string): number {
  return state.laneHeights?.[laneUiKey(track, key)] ?? LANE_HEIGHT;
}

/** Total height of `<TrackAutomationLanes track>` for a UI state (pure). */
export function automationHeight(state: HeightState, track: TrackId): number {
  // Closed: nothing (the toggle is an icon in the track header). Open: the parameter bar
  // and the lanes.
  if (!state.open.has(track)) return 0;
  let h = AUTOMATION_BAR_HEIGHT;
  for (const key of shownKeys(state, track)) h += laneHeightOf(state, track, key);
  return h;
}

/**
 * Current height of the automation slot of `track` (non-reactive; reads the store now).
 * For React code prefer `useAutomationHeight`, which re-renders when heights change.
 */
export function automationLaneHeights(track: TrackId): number {
  return automationHeight(useAutomationUi.getState(), track);
}

/**
 * `(track) => height` of the automation slot, for the arrangement's `layoutRows`. The
 * function identity changes whenever any height may have changed.
 */
export function useAutomationHeight(): (track: TrackId) => number {
  const open = useAutomationUi((s) => s.open);
  const shown = useAutomationUi((s) => s.shown);
  const laneHeights = useAutomationUi((s) => s.laneHeights);
  return useCallback((track: TrackId) => automationHeight({ open, shown, laneHeights }, track), [open, shown, laneHeights]);
}
