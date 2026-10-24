/**
 * Per-item selection inside timeline views: clips, notes and automation points, plus an
 * optional time-range selection.
 *
 * The app-wide "selected track" is NOT here: it lives in `@/state/selection`
 * (`useSelectionStore().selectTrack`). Views that want clicking a clip to also focus its
 * track call that themselves.
 *
 * Each kind is an independent set (selecting notes in the piano roll leaves the arrangement
 * clip selection alone). Sets are replaced (never mutated) on change, so they can be used
 * as React/zustand selector results. The app-wide `itemSelection` store prunes ids whose
 * entity disappears from the project mirror.
 */

import { useStore } from "zustand";
import { createStore, type StoreApi } from "zustand/vanilla";
import type { AutomationPointId, BeatRange, ClipId, NoteId, Project, TempoPointId } from "@/generated";
import { useProjectStore } from "@/state/projectStore";

/** Selectable item kinds and their id types. */
export interface SelectableIds {
  clip: ClipId;
  note: NoteId;
  automationPoint: AutomationPointId;
  /** The tempo lane's points. */
  tempoPoint: TempoPointId;
}
export type SelectableKind = keyof SelectableIds;

export const SELECTABLE_KINDS: ReadonlyArray<SelectableKind> = ["clip", "note", "automationPoint", "tempoPoint"];

/** The project table backing each kind (used for pruning). */
const TABLE: Record<SelectableKind, "clips" | "notes" | "automation_points" | "tempo_points"> = {
  clip: "clips",
  note: "notes",
  automationPoint: "automation_points",
  tempoPoint: "tempo_points",
};

/**
 * How a selection gesture combines with the current selection:
 * - `replace`: plain click / marquee;
 * - `add`: shift-click / shift-marquee;
 * - `toggle`: cmd/ctrl-click / cmd-marquee;
 * - `remove`: deselect the given ids.
 */
export type SelectMode = "replace" | "add" | "toggle" | "remove";

/** Selection mode from a pointer/keyboard event's modifiers (shift = add, cmd/ctrl = toggle). */
export function selectModeFromEvent(e: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }): SelectMode {
  if (e.metaKey || e.ctrlKey) return "toggle";
  if (e.shiftKey) return "add";
  return "replace";
}

/** Combine `current` with `ids` under `mode` (pure). Returns `current` when unchanged. */
export function combineSelection<Id>(current: ReadonlySet<Id>, ids: Iterable<Id>, mode: SelectMode): ReadonlySet<Id> {
  const list = [...ids];
  let next: Set<Id>;
  switch (mode) {
    case "replace":
      next = new Set(list);
      break;
    case "add":
      next = new Set(current);
      for (const id of list) next.add(id);
      break;
    case "remove":
      next = new Set(current);
      for (const id of list) next.delete(id);
      break;
    case "toggle":
      next = new Set(current);
      for (const id of list) {
        if (next.has(id)) next.delete(id);
        else next.add(id);
      }
      break;
  }
  return setsEqual(current, next) ? current : next;
}

function setsEqual<T>(a: ReadonlySet<T>, b: ReadonlySet<T>): boolean {
  if (a.size !== b.size) return false;
  for (const x of a) if (!b.has(x)) return false;
  return true;
}

export type SelectionSets = { readonly [K in SelectableKind]: ReadonlySet<SelectableIds[K]> };

export interface ItemSelectionState {
  selected: SelectionSets;
  /** Last clicked item per kind (anchor for shift-range selection by the view). */
  anchor: { readonly [K in SelectableKind]: SelectableIds[K] | null };
  /**
   * The kinds the last change selected together (`selectKinds`: a box over clips and
   * automation points); empty after any other change.
   */
  together: ReadonlyArray<SelectableKind>;
  /** Time-range selection (arrangement "select time"), or `null`. */
  timeRange: BeatRange | null;

  /** Select `ids` of `kind` combined with the current selection by `mode` (default replace). */
  select<K extends SelectableKind>(kind: K, ids: Iterable<SelectableIds[K]>, mode?: SelectMode): void;
  /**
   * Replace the selection of several kinds AT ONCE (one change notification): a box that
   * selects clips and automation points together. Kinds left out keep their selection.
   */
  selectKinds(sets: { readonly [K in SelectableKind]?: Iterable<SelectableIds[K]> }): void;
  /** Clear one kind, or everything (items and time range) when `kind` is omitted. */
  clear(kind?: SelectableKind): void;
  isSelected<K extends SelectableKind>(kind: K, id: SelectableIds[K]): boolean;
  setTimeRange(range: BeatRange | null): void;
  /** Drop ids whose entity no longer exists in `project` (`null` clears everything). */
  prune(project: Project | null): void;
}

export type ItemSelectionStore = StoreApi<ItemSelectionState>;

const EMPTY_SETS: SelectionSets = { clip: new Set(), note: new Set(), automationPoint: new Set(), tempoPoint: new Set() };
const EMPTY_ANCHORS = { clip: null, note: null, automationPoint: null, tempoPoint: null };

export function createItemSelectionStore(): ItemSelectionStore {
  return createStore<ItemSelectionState>()((set, get) => ({
    selected: EMPTY_SETS,
    anchor: EMPTY_ANCHORS,
    together: [],
    timeRange: null,

    select(kind, ids, mode = "replace") {
      const list = [...ids];
      const s = get();
      const current = s.selected[kind] as ReadonlySet<SelectableIds[typeof kind]>;
      const next = combineSelection(current, list, mode);
      const last = list[list.length - 1];
      const anchor = mode !== "remove" && last !== undefined ? last : s.anchor[kind];
      if (next === current && anchor === s.anchor[kind]) return;
      set({ selected: { ...s.selected, [kind]: next }, anchor: { ...s.anchor, [kind]: anchor }, together: [] });
    },

    selectKinds(sets) {
      const s = get();
      const selected = { ...s.selected } as Record<SelectableKind, ReadonlySet<string>>;
      let changed = false;
      for (const kind of SELECTABLE_KINDS) {
        const ids = sets[kind] as Iterable<string> | undefined;
        if (!ids) continue;
        const next = combineSelection(s.selected[kind] as ReadonlySet<string>, ids, "replace");
        if (next !== s.selected[kind]) {
          selected[kind] = next;
          changed = true;
        }
      }
      if (changed) set({ selected: selected as SelectionSets, together: Object.keys(sets) as SelectableKind[] });
    },

    clear(kind) {
      const s = get();
      if (kind === undefined) {
        if (SELECTABLE_KINDS.every((k) => s.selected[k].size === 0) && s.timeRange === null) return;
        set({ selected: EMPTY_SETS, anchor: EMPTY_ANCHORS, timeRange: null, together: [] });
        return;
      }
      if (s.selected[kind].size === 0 && s.anchor[kind] === null) return;
      set({ selected: { ...s.selected, [kind]: new Set() }, anchor: { ...s.anchor, [kind]: null }, together: [] });
    },

    isSelected(kind, id) {
      return (get().selected[kind] as ReadonlySet<SelectableIds[typeof kind]>).has(id);
    },

    setTimeRange(range) {
      set({ timeRange: range });
    },

    prune(project) {
      const s = get();
      if (!project) {
        get().clear();
        return;
      }
      let changed = false;
      const selected = { ...s.selected } as Record<SelectableKind, ReadonlySet<string>>;
      const anchor = { ...s.anchor } as Record<SelectableKind, string | null>;
      for (const kind of SELECTABLE_KINDS) {
        const table = project[TABLE[kind]] as Record<string, unknown>;
        const cur = s.selected[kind] as ReadonlySet<string>;
        let kept: Set<string> | null = null;
        for (const id of cur) {
          if (!(id in table)) {
            kept ??= new Set(cur);
            kept.delete(id);
          }
        }
        if (kept) {
          selected[kind] = kept;
          changed = true;
        }
        const a = s.anchor[kind];
        if (a !== null && !(a in table)) {
          anchor[kind] = null;
          changed = true;
        }
      }
      if (changed) set({ selected: selected as SelectionSets, anchor: anchor as ItemSelectionState["anchor"] });
    },
  }));
}

/**
 * Keep `store` pruned against the project mirror (removed clips/notes/points are
 * deselected, a new project clears everything). Returns the unsubscribe function.
 */
export function bindSelectionToProject(store: ItemSelectionStore): () => void {
  return useProjectStore.subscribe((s, prev) => {
    if (s.project !== prev.project) store.getState().prune(s.project);
  });
}

/** The app-wide item selection, pruned against the project mirror. */
export const itemSelection: ItemSelectionStore = createItemSelectionStore();
bindSelectionToProject(itemSelection);

/** Selected ids of one kind (re-renders when that set changes). */
export function useSelectedItems<K extends SelectableKind>(kind: K, store: ItemSelectionStore = itemSelection): ReadonlySet<SelectableIds[K]> {
  return useStore(store, (s) => s.selected[kind] as ReadonlySet<SelectableIds[K]>);
}

/** Whether one item is selected (re-renders only when that answer changes). */
export function useIsSelected<K extends SelectableKind>(kind: K, id: SelectableIds[K], store: ItemSelectionStore = itemSelection): boolean {
  return useStore(store, (s) => (s.selected[kind] as ReadonlySet<SelectableIds[K]>).has(id));
}

/** The current time-range selection. */
export function useTimeRangeSelection(store: ItemSelectionStore = itemSelection): BeatRange | null {
  return useStore(store, (s) => s.timeRange);
}
