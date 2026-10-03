/**
 * The insert marker (owner request, "like Ableton"): an edit cursor separate from the
 * playhead. It is the time selection at zero length (`store.ts`), so the selection store
 * holds both and placing one replaces the other.
 *
 * - A plain click on empty lane space (or a clip body) places it, snapped (Alt: free), on
 *   that track. Paste, split and the other edits that used to target the playhead target it
 *   (or the range selection); the playhead is only the fallback when there is neither.
 * - Clicks never move the playhead while playing. While stopped, placing the marker also
 *   locates the playhead there, so Play starts from it; placed while playing, the next stop
 *   locates there (`bindPlayFrom`). The ruler still locates/scrubs the playhead directly.
 */

import type { Beats, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { playheadBeats } from "@/timeline";
import { cmd, type EngineTransport } from "@/transport";
import { isMarker, rangeOf, useTimeSelection, type TimeRangeSelection } from "./store";

const playing = () => !!useProjectStore.getState().transport?.playing;

function locate(transport: EngineTransport, beats: Beats): void {
  transport.send(cmd("Transport", { type: "Locate", position: Math.max(0, beats) })).catch(() => {});
}

/**
 * Make song position `beats` where Play starts: locate now when stopped; when playing,
 * remember it for the next stop (the playing playhead never jumps).
 */
export function setPlayStart(transport: EngineTransport, beats: Beats): void {
  if (playing()) {
    useTimeSelection.setState({ playFrom: Math.max(0, beats) });
    return;
  }
  useTimeSelection.setState({ playFrom: null });
  locate(transport, beats);
}

/** Place the insert marker at `at` on `tracks` (replacing any time selection). */
export function placeInsertMarker(transport: EngineTransport | null, at: Beats, tracks: TrackId[]): void {
  const beats = Math.max(0, at);
  useTimeSelection.getState().setSelection({ start: beats, end: beats, tracks });
  if (transport) setPlayStart(transport, beats);
}

/** The insert marker, or null. */
export function insertMarker(): TimeRangeSelection | null {
  const sel = useTimeSelection.getState().selection;
  return isMarker(sel) ? sel : null;
}

/**
 * Where an edit without a range lands: the insert marker, else the range selection's start,
 * else the playhead (the fallback), with the track it points at (null for the playhead).
 */
export function insertPoint(): { at: Beats; track: TrackId | null } {
  const sel = useTimeSelection.getState().selection;
  if (sel) return { at: sel.start, track: sel.tracks[0] ?? null };
  return { at: playheadBeats(), track: null };
}

/** Where a paste lands: after a range selection (so pastes tile), at the marker, else the playhead. */
export function pasteTarget(sel: TimeRangeSelection | null = useTimeSelection.getState().selection): Beats {
  const range = rangeOf(sel);
  if (range) return range.end;
  return sel ? sel.start : playheadBeats();
}

/**
 * On stop, locate to the marker placed while playing (so Play starts from it, like Ableton).
 * Returns the unsubscribe.
 */
export function bindPlayFrom(transport: EngineTransport): () => void {
  return useProjectStore.subscribe((s, prev) => {
    const was = !!prev.transport?.playing;
    const now = !!s.transport?.playing;
    if (!was || now) return;
    const at = useTimeSelection.getState().playFrom;
    if (at === null) return;
    useTimeSelection.setState({ playFrom: null });
    locate(transport, at);
  });
}
