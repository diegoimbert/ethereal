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
import type {
  HistoryState,
  Patch,
  DeviceId,
  Project,
  ProjectSummary,
  TrackId,
  TransportState,
} from "@/generated";
import { applyPatchChanges } from "./entities";

export const EMPTY_HISTORY: HistoryState = { can_undo: false, can_redo: false, undo_label: null, redo_label: null };

/** Result of `applyPatch`. `"gap"`: a patch was missed; the caller should refetch (`Project::Get`). */
export type PatchResult = "applied" | "stale" | "gap" | "no-project";

export interface ProjectStoreState {
  project: Project | null;
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
   * base-131: the open project was opened without plugins (`Project::OpenSafe`) and they
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
  setProjects(projects: ReadonlyArray<ProjectSummary>): void;
  /** Update one project list entry (`Project::Saved`). */
  upsertProjectSummary(summary: ProjectSummary): void;
  setDirty(dirty: boolean): void;
  /** Back to the initial empty state (disconnect, tests). */
  reset(): void;
}

const INITIAL = {
  project: null,
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
        s.project = project;
        s.revision = null;
        s.history = opts?.history ?? EMPTY_HISTORY;
        // Armed tracks, the project list and the dirty flag come from their own events.
      });
    },

    clearProject() {
      set((s) => {
        s.project = null;
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
      set((s) => {
        // `s.project` is non-null: checked above, and zustand updates are synchronous.
        applyPatchChanges(s.project!, patch.changes);
        s.revision = patch.revision;
        s.history = patch.history;
      });
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
