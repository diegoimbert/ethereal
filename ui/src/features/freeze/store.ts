/**
 * Freeze / flatten / bounce / consolidate from the UI (CONTRACTS.md §12.3): command builders
 * and the render jobs in flight. Render jobs (`Freeze`, `Bounce`, audio `Consolidate`) run one
 * at a time in the engine: jobs asked for while one runs wait in a queue here, shown as
 * "queued" in their track headers. Progress comes from `Event::Freeze`; a failure is kept per
 * track (shown in its header until dismissed or the next job).
 */

import { create } from "zustand";
import type { Beats, Clip, Command, Event, FreezeCommand, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";

export type RenderKind = "Freeze" | "Bounce" | "Consolidate";

export interface RenderJob {
  job: string;
  kind: RenderKind;
  /** Tracks whose headers show the job. */
  tracks: TrackId[];
  /** 0..=1, `null` while queued. */
  progress: number | null;
}

interface FreezeUiState {
  jobs: Record<string, RenderJob>;
  /** Last failure per track (message). */
  errors: Record<TrackId, string>;
}

export const useFreezeUi = create<FreezeUiState>()(() => ({ jobs: {}, errors: {} }));

/** The job (running or queued) that shows in `track`'s header, if any. */
export function jobOfTrack(s: FreezeUiState, track: TrackId): RenderJob | undefined {
  for (const j of Object.values(s.jobs)) if (j.tracks.includes(track)) return j;
  return undefined;
}

export function dismissError(track: TrackId): void {
  useFreezeUi.setState((s) => {
    const { [track]: _, ...errors } = s.errors;
    return { errors };
  });
}

const errorText = (err: unknown): string =>
  err && typeof err === "object" && "error" in err
    ? String((err as { error: { message: string } }).error.message)
    : err instanceof Error
      ? err.message
      : String(err);

function setJob(job: string, patch: Partial<RenderJob> | null): void {
  useFreezeUi.setState((s) => {
    const cur = s.jobs[job];
    if (!cur) return s;
    const jobs = { ...s.jobs };
    if (patch) jobs[job] = { ...cur, ...patch };
    else delete jobs[job];
    return { jobs };
  });
}

function fail(tracks: TrackId[], message: string): void {
  useFreezeUi.setState((s) => {
    const errors = { ...s.errors };
    for (const t of tracks) errors[t] = message;
    return { errors };
  });
}

/** Resolvers of the jobs waiting for their end event. */
const waiting = new Map<string, () => void>();
const listening = new WeakSet<EngineTransport>();

function listen(transport: EngineTransport): void {
  if (listening.has(transport)) return;
  listening.add(transport);
  transport.onEvent((e: Event) => {
    if (e.type !== "Freeze") return;
    const ev = e.event;
    const job = useFreezeUi.getState().jobs[ev.job];
    if (!job) return;
    switch (ev.type) {
      case "Progress":
        setJob(ev.job, { progress: ev.progress });
        return;
      case "Failed":
        fail(job.tracks, ev.message);
        break;
      case "Done":
      case "Cancelled":
        break;
    }
    setJob(ev.job, null);
    waiting.get(ev.job)?.();
    waiting.delete(ev.job);
  });
}

/** Jobs run one after another (the engine runs one render at a time). */
let queue: Promise<void> = Promise.resolve();

/**
 * Queue a render job: `command(job)` is sent when the previous jobs ended; the returned
 * promise resolves when this one ends (done, failed or cancelled).
 */
export function runRenderJob(
  transport: EngineTransport,
  kind: RenderKind,
  tracks: TrackId[],
  command: (job: string) => Command,
): Promise<void> {
  listen(transport);
  const job = newId();
  useFreezeUi.setState((s) => {
    const errors = { ...s.errors };
    for (const t of tracks) delete errors[t];
    return { jobs: { ...s.jobs, [job]: { job, kind, tracks, progress: null } }, errors };
  });
  const run = async () => {
    // Cancelled while queued.
    if (!useFreezeUi.getState().jobs[job]) return;
    const ended = new Promise<void>((resolve) => waiting.set(job, resolve));
    try {
      const reply = await transport.send(command(job));
      if (reply.type !== "RenderStarted") {
        // Instantaneous (MIDI-only consolidate).
        waiting.delete(job);
        setJob(job, null);
        return;
      }
      // Progress may already have arrived with the reply.
      setJob(job, { progress: useFreezeUi.getState().jobs[job]?.progress ?? 0 });
    } catch (err) {
      waiting.delete(job);
      setJob(job, null);
      fail(tracks, errorText(err));
      return;
    }
    await ended;
  };
  const done = queue.then(run);
  queue = done.catch(() => {});
  return done;
}

/** Forget every job (a new transport/connection: their events won't come). */
export function resetRenderJobs(): void {
  for (const resolve of waiting.values()) resolve();
  waiting.clear();
  queue = Promise.resolve();
  useFreezeUi.setState({ jobs: {}, errors: {} });
}

/** Cancel a running or queued job. */
export async function cancelRenderJob(transport: EngineTransport, job: RenderJob): Promise<void> {
  if (job.progress === null) {
    setJob(job.job, null);
    return;
  }
  await transport.send(cmd("Freeze", { type: "Cancel", job: job.job })).catch(() => {});
}

// ─── Commands ─────────────────────────────────────────────────────────────────────────────

const freeze = (c: FreezeCommand): Command => cmd("Freeze", c);

/** Tracks that can be frozen (audio/MIDI, not frozen, with clips). */
export function canFreeze(track: TrackId): boolean {
  const p = useProjectStore.getState().project;
  const t = p?.tracks[track];
  if (!p || !t || t.freeze || (t.kind !== "Audio" && t.kind !== "Midi")) return false;
  return Object.values(p.clips).some((c) => c.track === track);
}

export function freezeTracks(transport: EngineTransport, tracks: TrackId[]): Promise<void[]> {
  return Promise.all(
    tracks.map((track) =>
      runRenderJob(transport, "Freeze", [track], (job) => freeze({ type: "Freeze", job, track, media: newId() })),
    ),
  );
}

async function sendOrRecord(transport: EngineTransport, tracks: TrackId[], command: Command): Promise<void> {
  try {
    await transport.send(command);
  } catch (err) {
    fail(tracks, errorText(err));
  }
}

export function unfreezeTracks(transport: EngineTransport, tracks: TrackId[]): Promise<void[]> {
  return Promise.all(tracks.map((track) => sendOrRecord(transport, [track], freeze({ type: "Unfreeze", track }))));
}

export function flattenTracks(transport: EngineTransport, tracks: TrackId[]): Promise<void[]> {
  return Promise.all(
    tracks.map((track) =>
      sendOrRecord(transport, [track], freeze({ type: "Flatten", track, clip: newId(), new_track: newId() })),
    ),
  );
}

/** `[start, end)` spanned by `clips`. */
export function spanOf(clips: ReadonlyArray<Clip>): { start: Beats; end: Beats } {
  return {
    start: Math.min(...clips.map((c) => c.start)),
    end: Math.max(...clips.map((c) => c.start + c.length)),
  };
}

function byTrack(clips: ReadonlyArray<Clip>): Map<TrackId, Clip[]> {
  const out = new Map<TrackId, Clip[]>();
  for (const c of clips) out.set(c.track, [...(out.get(c.track) ?? []), c]);
  return out;
}

/**
 * Bounce the span of the selected clips of each track: to a new audio track below it
 * (post-chain; the source is muted), or in place (audio clips, pre-chain).
 */
export function bounceClips(
  transport: EngineTransport,
  clips: ReadonlyArray<Clip>,
  target: "NewTrack" | "InPlace",
): Promise<void[]> {
  return Promise.all(
    [...byTrack(clips)].map(([track, own]) => {
      const { start, end } = spanOf(own);
      return runRenderJob(transport, "Bounce", [track], (job) =>
        freeze({
          type: "Bounce",
          job,
          track,
          start,
          end,
          include_chain: target === "NewTrack",
          media: newId(),
          target: target === "NewTrack" ? { type: "NewTrack", track: newId(), clip: newId() } : { type: "InPlace", clip: newId() },
        }),
      );
    }),
  );
}

/** Join the selected clips' span into one clip per track (MIDI instantly, audio rendered). */
export function consolidateClips(transport: EngineTransport, clips: ReadonlyArray<Clip>): Promise<void> {
  const { start, end } = spanOf(clips);
  const tracks = [...byTrack(clips).keys()];
  return runRenderJob(transport, "Consolidate", tracks, (job) =>
    freeze({
      type: "Consolidate",
      job,
      tracks,
      start,
      end,
      seed_clips: newId(),
      seed_notes: newId(),
      seed_media: newId(),
    }),
  );
}
