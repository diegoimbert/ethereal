/**
 * What is still loading while a project opens (#260), per track and per clip: a loading
 * track's header and lane look unavailable (dimmed, desaturated, controls inactive), and
 * so do its clips that aren't ready, until they are.
 *
 * A track is loading while one of its plugin devices is still to load (`PluginEvent::Loading`,
 * the same count as its header's bar; all its clips wait for them), or while one of its
 * audio clips' media still decodes (`LoadProgress.media_pending`): then only those clips
 * wait, and the track is ready as soon as its own media are, before the rest of the project.
 */
import { useMemo } from "react";
import type { ClipId, DeviceId, LoadProgress, MediaId, Project, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { usePluginEvents, usePluginStore } from "./pluginStore";

/** What is still loading (see the module doc). */
export interface Loading {
  tracks: ReadonlySet<TrackId>;
  /** Clips not ready: all of a track whose plugins load, audio clips whose media decode. */
  clips: ReadonlySet<ClipId>;
  /** Per track with audio clips, while media decode: `[loaded, total]` of its media. */
  media: ReadonlyMap<TrackId, readonly [number, number]>;
}

const NO_TRACKS: ReadonlySet<TrackId> = new Set();
const NO_CLIPS: ReadonlySet<ClipId> = new Set();
const NOTHING: Loading = { tracks: NO_TRACKS, clips: NO_CLIPS, media: new Map() };

/**
 * The media still decoding: `null` when none; `"all"` when the engine says some do without
 * saying which (an older engine): then every audio clip waits, as before.
 */
function pendingMedia(load: LoadProgress | null | undefined): ReadonlySet<MediaId> | "all" | null {
  if (!load || load.media_loaded >= load.media_total) return null;
  return load.media_pending ? new Set(load.media_pending) : "all";
}

/** What is still loading (pure): see the module doc. */
export function loadingOf(project: Project | null, pendingPlugins: ReadonlyArray<DeviceId>, load: LoadProgress | null | undefined): Loading {
  if (!project) return NOTHING;
  const pending = pendingMedia(load);
  if (pendingPlugins.length === 0 && !pending) return NOTHING;
  const tracks = new Set<TrackId>();
  for (const id of pendingPlugins) {
    const d = project.devices[id];
    if (d) tracks.add(d.track);
  }
  const clips = new Set<ClipId>();
  const media = new Map<TrackId, readonly [number, number]>();
  // Per track: its media, and those still decoding.
  const perTrack = new Map<TrackId, [Set<MediaId>, Set<MediaId>]>();
  for (const c of Object.values(project.clips)) {
    if (tracks.has(c.track)) clips.add(c.id);
    if (!pending || c.content.type !== "Audio") continue;
    const m = c.content.media;
    let t = perTrack.get(c.track);
    if (!t) perTrack.set(c.track, (t = [new Set(), new Set()]));
    t[0].add(m);
    if (pending === "all" || pending.has(m)) {
      t[1].add(m);
      clips.add(c.id);
    }
  }
  for (const [track, [all, left]] of perTrack) {
    if (left.size === 0) continue;
    tracks.add(track);
    media.set(track, [all.size - left.size, all.size]);
  }
  if (tracks.size === 0) return NOTHING;
  return { tracks, clips, media };
}

/** The tracks still loading (pure): see the module doc. */
export function loadingTracksOf(project: Project | null, pendingPlugins: ReadonlyArray<DeviceId>, load: LoadProgress | null | undefined): ReadonlySet<TrackId> {
  return loadingOf(project, pendingPlugins, load).tracks;
}

let memo: { project: Project | null; pending: ReadonlyArray<DeviceId>; load: LoadProgress | null | undefined; out: Loading } | null = null;

/** {@link loadingOf}, computed once per change of its inputs (every header asks). */
function loadingCached(project: Project | null, pending: ReadonlyArray<DeviceId>, load: LoadProgress | null | undefined): Loading {
  if (!memo || memo.project !== project || memo.pending !== pending || memo.load !== load) {
    memo = { project, pending, load, out: loadingOf(project, pending, load) };
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
  const key = useProjectStore((s) => keyOf(loadingCached(s.project, pending, s.transport?.loading).tracks));
  return useMemo(() => (key === "" ? NO_TRACKS : new Set(key.split(",") as TrackId[])), [key]);
}

/** Whether `track` is still loading. */
export function useTrackLoading(track: TrackId): boolean {
  usePluginEvents();
  const pending = usePluginStore((s) => s.loading);
  return useProjectStore((s) => loadingCached(s.project, pending, s.transport?.loading).tracks.has(track));
}

/** `"loaded/total"` of `track`'s own media while some of them decode, else null. */
export function useTrackMediaLoading(track: TrackId): string | null {
  usePluginEvents();
  const pending = usePluginStore((s) => s.loading);
  return useProjectStore((s) => {
    const m = loadingCached(s.project, pending, s.transport?.loading).media.get(track);
    return m ? `${m[0]}/${m[1]}` : null;
  });
}

/** What is still loading, outside React (the arranger canvas). */
export function loadingNow(): Loading {
  const s = useProjectStore.getState();
  return loadingCached(s.project, usePluginStore.getState().loading, s.transport?.loading);
}

/** The tracks still loading, outside React. */
export function loadingTracksNow(): ReadonlySet<TrackId> {
  return loadingNow().tracks;
}

/** Calls `fn` when what is loading may have changed. Returns the unsubscription. */
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
