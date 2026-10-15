/**
 * The app's live recording view (`LiveRecording`), fed from the transport's recording and
 * media events. Any component that shows it calls `useLiveRecordingFeed(transport)` (the
 * subscription is shared and ref-counted per transport).
 */

import { useCallback, useEffect, useSyncExternalStore } from "react";
import type { ClipId, MediaId, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { LiveRecording, type LiveTrackView } from "./liveModel";

function mediaOfClip(id: ClipId): MediaId | null {
  const c = useProjectStore.getState().project?.clips[id];
  return c?.content.type === "Audio" ? c.content.media : null;
}

export const liveRecording = new LiveRecording(mediaOfClip);

const feeds = new Map<EngineTransport, { count: number; off: () => void }>();

/** Feed `liveRecording` from `transport` (ref-counted; returns the release function). */
export function attachLiveFeed(transport: EngineTransport, model: LiveRecording = liveRecording): () => void {
  let feed = feeds.get(transport);
  if (!feed) {
    const off = transport.onEvent((e) => {
      if (e.type === "Recording") model.apply(e.event);
      else if (e.type === "Media" && e.event.type === "PeaksReady") model.peaksReady(e.event.media);
      else if (e.type === "ProjectLoaded") model.reset();
    });
    feed = { count: 0, off };
    feeds.set(transport, feed);
  }
  feed.count += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    const f = feeds.get(transport);
    if (!f) return;
    f.count -= 1;
    if (f.count === 0) {
      f.off();
      feeds.delete(transport);
    }
  };
}

export function useLiveRecordingFeed(transport: EngineTransport | null | undefined): void {
  useEffect(() => (transport ? attachLiveFeed(transport) : undefined), [transport]);
}

/** The live view of one track (re-renders on its changes only). */
export function useLiveTrack(track: TrackId): LiveTrackView {
  const subscribe = useCallback((l: () => void) => liveRecording.subscribe(l), []);
  return useSyncExternalStore(subscribe, () => liveRecording.view(track));
}
