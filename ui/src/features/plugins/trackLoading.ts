/**
 * Which tracks are still loading while a project opens (#260): their header, lane and clips
 * look unavailable (dimmed, desaturated, controls inactive) until they are ready.
 *
 * A track is loading while one of its plugin devices is still to load (`PluginEvent::Loading`,
 * the same count as its header's bar), or, while the load decodes media
 * (`TransportState.loading`: no per-file progress), while it has audio clips.
 */
import { useMemo } from "react";
import type { DeviceId, LoadProgress, Project, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { usePluginEvents, usePluginStore } from "./pluginStore";

const NONE: ReadonlySet<TrackId> = new Set();

/** The tracks still loading (pure): see the module doc. */
export function loadingTracksOf(project: Project | null, pendingPlugins: ReadonlyArray<DeviceId>, load: LoadProgress | null | undefined): ReadonlySet<TrackId> {
  if (!project) return NONE;
  const media = !!load && load.media_loaded < load.media_total;
  if (pendingPlugins.length === 0 && !media) return NONE;
  const out = new Set<TrackId>();
  for (const id of pendingPlugins) {
    const d = project.devices[id];
    if (d) out.add(d.track);
  }
  if (media) {
    for (const c of Object.values(project.clips)) if (c.content.type === "Audio") out.add(c.track);
  }
  return out.size === 0 ? NONE : out;
}

let memo: { project: Project | null; pending: ReadonlyArray<DeviceId>; media: boolean; out: ReadonlySet<TrackId> } | null = null;

/** {@link loadingTracksOf}, computed once per change of its inputs (every header asks). */
function loadingTracksCached(project: Project | null, pending: ReadonlyArray<DeviceId>, load: LoadProgress | null | undefined): ReadonlySet<TrackId> {
  const media = !!load && load.media_loaded < load.media_total;
  if (!memo || memo.project !== project || memo.pending !== pending || memo.media !== media) {
    memo = { project, pending, media, out: loadingTracksOf(project, pending, load) };
  }
  return memo.out;
}

/** A stable key of a set of tracks (for store selectors). */
function keyOf(set: ReadonlySet<TrackId>): string {
  return [...set].sort().join(",");
}

/** The tracks still loading (re-renders only when the set changes). */
export function useLoadingTracks(): ReadonlySet<TrackId> {
  usePluginEvents();
  const pending = usePluginStore((s) => s.loading);
  const key = useProjectStore((s) => keyOf(loadingTracksCached(s.project, pending, s.transport?.loading)));
  return useMemo(() => (key === "" ? NONE : new Set(key.split(",") as TrackId[])), [key]);
}

/** Whether `track` is still loading. */
export function useTrackLoading(track: TrackId): boolean {
  usePluginEvents();
  const pending = usePluginStore((s) => s.loading);
  return useProjectStore((s) => loadingTracksCached(s.project, pending, s.transport?.loading).has(track));
}

/** The tracks still loading, outside React (the arranger canvas). */
export function loadingTracksNow(): ReadonlySet<TrackId> {
  const s = useProjectStore.getState();
  return loadingTracksCached(s.project, usePluginStore.getState().loading, s.transport?.loading);
}

/** Calls `fn` when the set of loading tracks may have changed. Returns the unsubscription. */
export function subscribeLoadingTracks(fn: () => void): () => void {
  const a = usePluginStore.subscribe((s, p) => {
    if (s.loading !== p.loading) fn();
  });
  const b = useProjectStore.subscribe((s, p) => {
    if (s.transport?.loading !== p.transport?.loading) fn();
  });
  return () => {
    a();
    b();
  };
}
