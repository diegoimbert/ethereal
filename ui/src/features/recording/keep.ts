// Which tracks are armed and whether punch is on are engine runtime state (not in the
// project, not undoable, and they differ per computer in a live session). They are kept on
// this computer per project (localStorage `eth.rec.<project id>`) and set again when the
// project opens: a reopen or a reload finds the same arms and punch.
import { useContext, useEffect } from "react";
import type { ProjectId, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, TransportContext, type EngineTransport } from "@/transport";

export interface KeptRecordingSetup {
  armed: TrackId[];
  punch: boolean;
}

const PREFIX = "eth.rec.";
export const recordingSetupKey = (project: ProjectId) => `${PREFIX}${project}`;

export function loadRecordingSetup(project: ProjectId): KeptRecordingSetup | null {
  try {
    const raw = localStorage.getItem(recordingSetupKey(project));
    if (!raw) return null;
    const v = JSON.parse(raw) as { armed?: unknown; punch?: unknown };
    return {
      armed: Array.isArray(v.armed) ? v.armed.filter((t): t is TrackId => typeof t === "string") : [],
      punch: v.punch === true,
    };
  } catch {
    return null;
  }
}

function saveRecordingSetup(project: ProjectId, setup: KeptRecordingSetup): void {
  try {
    if (setup.armed.length === 0 && !setup.punch) localStorage.removeItem(recordingSetupKey(project));
    else localStorage.setItem(recordingSetupKey(project), JSON.stringify(setup));
  } catch {
    /* not kept (private mode, quota) */
  }
}

/**
 * Keep the arms and punch of each project opened through `transport`, and set them again
 * when it opens. Returns the unbind.
 */
export function bindRecordingSetup(transport: EngineTransport): () => void {
  let project: ProjectId | null = null;
  /** Restoring the opened project's setup: its changes (the load's own "nothing armed") aren't kept. */
  let ready = false;
  let punch = false;
  let disposed = false;

  const save = () => {
    if (!ready || !project) return;
    saveRecordingSetup(project, { armed: [...useProjectStore.getState().armedTracks], punch });
  };

  const opened = async (id: ProjectId) => {
    project = id;
    ready = false;
    const kept = loadRecordingSetup(id);
    // After the load's own events (it clears the arms).
    await Promise.resolve();
    const tracks = useProjectStore.getState().project?.tracks ?? {};
    const sends: Promise<unknown>[] = [];
    for (const track of kept?.armed ?? []) {
      const t = tracks[track];
      // Tracks deleted since are dropped.
      if (t && (t.kind === "Audio" || t.kind === "Midi")) sends.push(transport.send(cmd("Recording", { type: "Arm", track, armed: true, exclusive: false })));
    }
    // Punch is engine-wide: set it for this project (off unless kept on).
    sends.push(transport.send(cmd("Recording", { type: "SetPunch", enabled: kept?.punch ?? false })));
    await Promise.allSettled(sends);
    if (disposed || project !== id) return;
    ready = true;
    save();
  };

  const unsubProject = useProjectStore.subscribe((s, prev) => {
    const id = s.project?.id ?? null;
    if (id !== (prev.project?.id ?? null) || (id && id !== project)) {
      if (id) void opened(id);
      else {
        project = null;
        ready = false;
      }
    } else if (s.armedTracks !== prev.armedTracks) save();
  });
  const unsubEvents = transport.onEvent((e) => {
    if (e.type === "Recording" && e.event.type === "PunchChanged") {
      punch = e.event.enabled;
      save();
    }
  });
  const current = useProjectStore.getState().project?.id;
  if (current) void opened(current);
  return () => {
    disposed = true;
    unsubProject();
    unsubEvents();
  };
}

/** Mounted by the app shells (desktop and mobile): keep the recording setup per project. */
export function useKeepRecordingSetup(): void {
  const transport = useContext(TransportContext)?.transport ?? null;
  useEffect(() => (transport ? bindRecordingSetup(transport) : undefined), [transport]);
}
