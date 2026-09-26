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
 * - session clip play states (`Event::Session`);
 * - record-armed tracks (`Event::Recording { ArmChanged }`; runtime state, not document);
 * - the engine-side project list and the current project's dirty flag (`Event::Project`).
 * High-rate data (playhead, meters) lives in `./playhead.ts`, outside React state.
 *
 * `TransportProvider` wires the transport to this store; tests can drive it directly.
 */

import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type {
  ClipId,
  ClipPlayState,
  ClipStateChange,
  HistoryState,
  Patch,
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
  /** Play state of session clips that aren't `Stopped` (absent = stopped). */
  sessionStates: Record<ClipId, ClipPlayState>;
  /** Record-armed tracks (engine runtime state). */
  armedTracks: TrackId[];
  /** Projects in the engine-side store, as last reported (`Project::ListChanged`). */
  projects: ProjectSummary[];
  /** The current project has unsaved changes. */
  dirty: boolean;

  /** Replace the whole document (connect, `ProjectLoaded`, refetch). */
  loadProject(project: Project, opts?: { history?: HistoryState }): void;
  /** Apply an `Event::Patch`. Ignores stale revisions; reports gaps without applying. */
  applyPatch(patch: Patch): PatchResult;
  setTransport(state: TransportState): void;
  applySessionChanges(changes: ReadonlyArray<ClipStateChange>): void;
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
  sessionStates: {},
  armedTracks: [],
  projects: [],
  dirty: false,
} satisfies Partial<ProjectStoreState>;

export const useProjectStore = create<ProjectStoreState>()(
  immer((set, get) => ({
    ...INITIAL,

    loadProject(project, opts) {
      set((s) => {
        s.project = project;
        s.revision = null;
        s.history = opts?.history ?? EMPTY_HISTORY;
        // A new document invalidates session slot states (clip ids may be gone). Armed
        // tracks, the project list and the dirty flag come from their own events.
        s.sessionStates = {};
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
        // Drop play states of removed clips.
        for (const c of patch.changes) {
          if (c.type === "Remove" && c.key.type === "Clip") delete s.sessionStates[c.key.id];
        }
      });
      return "applied";
    },

    setTransport(state) {
      set((s) => {
        s.transport = state;
      });
    },

    applySessionChanges(changes) {
      set((s) => {
        for (const c of changes) {
          if (c.state === "Stopped") delete s.sessionStates[c.clip];
          else s.sessionStates[c.clip] = c.state;
        }
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
