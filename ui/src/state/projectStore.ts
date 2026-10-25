/**
 * The UI mirror of the engine's document (zustand + immer).
 *
 * The engine owns the document; this store only ever changes by applying what the engine
 * sends: a full `Project` (connect / `Event::ProjectLoaded` / refetch) or `Event::Patch`
 * (whole-entity upserts/removes, applied with no domain logic). Features never write to
 * `project` directly: they `transport.send(...)` a command and render what comes back.
 *
 * Also holds low-rate engine state pushed as events:
 * - `TransportState` (`Event::Transport`);
 * - record-armed tracks (`Event::Recording { ArmChanged }`; runtime state, not document);
 * - the engine-side project list and the current project's dirty flag (`Event::Project`).
 * High-rate data (playhead, meters) lives in `./playhead.ts`, outside React state.
 *
 * `TransportProvider` wires the transport to this store; tests can drive it directly.
 */

import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { HistoryState, Patch, DeviceId, Project, ProjectSummary, TrackId, TransportState } from "@/generated";
import { withPatchChanges } from "./entities";
import { carryGroups } from "./groups";
import { noteTable } from "./noteTable";

export const EMPTY_HISTORY: HistoryState = { can_undo: false, can_redo: false, undo_label: null, redo_label: null, undo_id: null, redo_id: null };

/** Result of `applyPatch`. `"gap"`: a patch was missed; the caller should refetch (`Project::Get`). */
export type PatchResult = "applied" | "stale" | "gap" | "no-project";

export interface ProjectStoreState {
  project: Project | null;
  /**
   * The object last given to `loadProject` (`project` is a copy of it with its notes
   * sharded): lets a caller tell whether its load is still the current one.
   */
  loadedFrom: Project | null;
  /**
   * Revision of the last applied patch. `null` right after a (re)load: the full project
   * carries no revision, so the next patch is accepted as-is and sets it.
   */
  revision: number | null;
  history: HistoryState;
  transport: TransportState | null;
  /** Record-armed tracks (engine runtime state). */
  armedTracks: TrackId[];
  /** Projects in the engine-side store, as last reported (`Project::ListChanged`). */
  projects: ProjectSummary[];
  /** The current project has unsaved changes. */
  dirty: boolean;
  /**
   * The open project was opened without plugins (`Project::OpenSafe`) and they
   * are not loaded yet (`Event::Project { SafeMode }`).
   */
  safe: boolean;
  /** In safe mode: the plugin devices held as bypassed placeholders. */
  safeMode: DeviceId[];

  /** Replace the whole document (connect, `ProjectLoaded`, refetch). Keeps `safeMode`. */
  loadProject(project: Project, opts?: { history?: HistoryState }): void;
  /** No project is open (connected to an engine that has none: launch). */
  clearProject(): void;
  setSafeMode(active: boolean, devices: ReadonlyArray<DeviceId>): void;
  /** Apply an `Event::Patch`. Ignores stale revisions; reports gaps without applying. */
  applyPatch(patch: Patch): PatchResult;
  setTransport(state: TransportState): void;
  setArmedTracks(armed: ReadonlyArray<TrackId>): void;
  /** The undo/redo state read from the engine (`History::State`, after a connect). */
  setHistory(history: HistoryState): void;
  setProjects(projects: ReadonlyArray<ProjectSummary>): void;
  /** Update one project list entry (`Project::Saved`). */
  upsertProjectSummary(summary: ProjectSummary): void;
  setDirty(dirty: boolean): void;
  /** Back to the initial empty state (disconnect, tests). */
  reset(): void;
}

const INITIAL = {
  project: null,
  loadedFrom: null,
  revision: null,
  history: EMPTY_HISTORY,
  transport: null,
  armedTracks: [],
  projects: [],
  dirty: false,
  safe: false,
  safeMode: [],
} satisfies Partial<ProjectStoreState>;

export const useProjectStore = create<ProjectStoreState>()(
  immer((set, get) => ({
    ...INITIAL,

    loadProject(project, opts) {
      set((s) => {
        // Notes sharded (`./noteTable`): patches copy only the buckets they touch.
        s.project = { ...project, notes: noteTable(project.notes) };
        s.loadedFrom = project;
        s.revision = null;
        s.history = opts?.history ?? EMPTY_HISTORY;
        // Armed tracks, the project list and the dirty flag come from their own events.
      });
    },

    clearProject() {
      set((s) => {
        s.project = null;
        s.loadedFrom = null;
        s.revision = null;
        s.history = EMPTY_HISTORY;
        s.dirty = false;
        s.safe = false;
        s.safeMode = [];
      });
    },

    setSafeMode(active, devices) {
      set((s) => {
        s.safe = active;
        s.safeMode = [...devices];
      });
    },

    applyPatch(patch) {
      const { project, revision } = get();
      if (!project) return "no-project";
      if (revision !== null && patch.revision <= revision) return "stale";
      if (revision !== null && patch.revision > revision + 1) return "gap";
      // Copy-on-write without immer: a big table's immer draft costs its copy plus a scan
      // of every entry (a one-note edit in a 262k-note project: ~150 ms).
      set({ project: withPatchChanges(project, patch.changes), revision: patch.revision, history: patch.history });
      // The group-bys (notes by clip, ...) follow without regrouping every entity.
      const next = get().project;
      if (next) carryGroups(project, next, patch.changes);
      return "applied";
    },

    setTransport(state) {
      set((s) => {
        s.transport = state;
      });
    },

    setArmedTracks(armed) {
      set((s) => {
        s.armedTracks = [...armed];
      });
    },

    setHistory(history) {
      set((s) => {
        s.history = history;
      });
    },

    setProjects(projects) {
      set((s) => {
        s.projects = [...projects];
      });
    },

    upsertProjectSummary(summary) {
      set((s) => {
        const i = s.projects.findIndex((p) => p.id === summary.id);
        if (i >= 0) s.projects[i] = summary;
        else s.projects.unshift(summary);
      });
    },

    setDirty(dirty) {
      set((s) => {
        s.dirty = dirty;
      });
    },

    reset() {
      set(() => ({ ...INITIAL }));
    },
  })),
);
