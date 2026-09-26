/**
 * The UI mirror of the engine's document (zustand + immer).
 *
 * The engine owns the document; this store only ever changes by applying what the engine
 * sends: a full `Project` (connect / `Event::ProjectLoaded` / refetch) or `Event::Patch`
 * (whole-entity upserts/removes, applied with no domain logic). Features never write to
 * `project` directly: they `transport.send(...)` a command and render what comes back.
 *
 * Also holds low-rate engine state pushed as events: `TransportState` (`Event::Transport`)
 * and session clip play states (`Event::Session`). High-rate data (playhead, meters) lives
 * in `./playhead.ts`, outside React state.
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
  TransportState,
} from "@/generated";
import { applyPatchChanges } from "./entities";

export const EMPTY_HISTORY: HistoryState = { can_undo: false, can_redo: false, undo_label: null, redo_label: null };

/** Result of `applyPatch`. `"gap"`: a patch was missed; the caller should refetch (`Project::Get`). */
export type PatchResult = "applied" | "stale" | "gap" | "no-project";

export interface ProjectStoreState {
  project: Project | null;
  /** File path of the loaded project, if any (from `Event::ProjectLoaded`). */
  path: string | null;
  /**
   * Revision of the last applied patch. `null` right after a (re)load: the full project
   * carries no revision, so the next patch is accepted as-is and sets it.
   */
  revision: number | null;
  history: HistoryState;
  transport: TransportState | null;
  /** Play state of session clips that aren't `Stopped` (absent = stopped). */
  sessionStates: Record<ClipId, ClipPlayState>;

  /** Replace the whole document (connect, `ProjectLoaded`, refetch). */
  loadProject(project: Project, opts?: { path?: string | null; history?: HistoryState }): void;
  /** Apply an `Event::Patch`. Ignores stale revisions; reports gaps without applying. */
  applyPatch(patch: Patch): PatchResult;
  setTransport(state: TransportState): void;
  applySessionChanges(changes: ReadonlyArray<ClipStateChange>): void;
  /** Back to the initial empty state (disconnect, tests). */
  reset(): void;
}

const INITIAL = {
  project: null,
  path: null,
  revision: null,
  history: EMPTY_HISTORY,
  transport: null,
  sessionStates: {},
} satisfies Partial<ProjectStoreState>;

export const useProjectStore = create<ProjectStoreState>()(
  immer((set, get) => ({
    ...INITIAL,

    loadProject(project, opts) {
      set((s) => {
        s.project = project;
        s.path = opts?.path ?? null;
        s.revision = null;
        s.history = opts?.history ?? EMPTY_HISTORY;
        // A new document invalidates session slot states (clip ids may be gone).
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

    reset() {
      set(() => ({ ...INITIAL }));
    },
  })),
);
