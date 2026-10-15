/**
 * The insert marker and the SELECTION CURSOR (owner requests, "like Ableton"): an edit cursor
 * separate from the playhead. The marker is the time selection at zero length (`store.ts`),
 * so the selection store holds both and placing one replaces the other; the cursor
 * (`useTimeSelection.cursor`) is the start of the last selection made anywhere (arrangement
 * marker or range, piano-roll marker or section, a clip click), and outlives it.
 *
 * - A plain click on empty lane space (or a clip body) places the marker, snapped (Alt:
 *   free), on that track. Paste, split, duplicate, insert and the other edits that used to
 *   target the playhead target it (or the range selection), else the cursor; NEVER the
 *   playhead. The playhead is only "where playback is / will start".
 * - Clicks never move the playhead while playing. While stopped, setting the cursor (any
 *   selection) also locates the playhead to it, so Play starts there; set while playing, the
 *   next stop locates there (`bindPlayFrom`). The ruler still locates / scrubs the playhead
 *   directly (Ableton's scrub area), without moving the cursor.
 * - Space (`playOrStopToStart`): play; while playing, stop and return to the selection
 *   (insert marker / section start), else to where playback started. Shift+Space (`playOrStopHere`): play; while playing, stop where the playhead is.
 * - The Stop button (`stopButton`): stops; already stopped, returns to the cursor, and when
 *   already there, to the project start (Ableton's double stop).
 */

import type { Beats, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { playheadBeats } from "@/timeline";
import { cmd, type EngineTransport } from "@/transport";
import { isMarker, rangeOf, useTimeSelection, type TimeRangeSelection } from "./store";

const playing = () => !!useProjectStore.getState().transport?.playing;

/** Positions closer than this are "the same place" for the Stop button. */
const SAME_PLACE = 1e-6;

function locate(transport: EngineTransport, beats: Beats): void {
  transport.send(cmd("Transport", { type: "Locate", position: Math.max(0, beats) })).catch(() => {});
}

/**
 * Set the selection cursor to song position `beats` and make it where Play starts: locate
 * now when stopped; when playing, remember it for the next stop (the playing playhead never
 * jumps).
 */
export function setPlayStart(transport: EngineTransport, beats: Beats): void {
  const at = Math.max(0, beats);
  if (playing()) {
    useTimeSelection.setState({ playFrom: at, cursor: at });
    return;
  }
  useTimeSelection.setState({ playFrom: null, cursor: at });
  locate(transport, at);
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

/** The selection cursor: the start of the last selection made (0 before any). */
export function selectionCursor(): Beats {
  return useTimeSelection.getState().cursor;
}

/**
 * Where an edit without a range lands: the insert marker, else the range selection's start,
 * else the selection cursor, with the track it points at (null for the bare cursor).
 */
export function insertPoint(): { at: Beats; track: TrackId | null } {
  const sel = useTimeSelection.getState().selection;
  if (sel) return { at: sel.start, track: sel.tracks[0] ?? null };
  return { at: selectionCursor(), track: null };
}

/** Where a paste lands: after a range selection (so pastes tile), at the marker, else the cursor. */
export function pasteTarget(sel: TimeRangeSelection | null = useTimeSelection.getState().selection): Beats {
  const range = rangeOf(sel);
  if (range) return range.end;
  return sel ? sel.start : selectionCursor();
}

/**
 * After an edit that inserted material up to `at` (a paste): move the cursor there, as the
 * insert marker (on the selection's tracks) when there is a time selection, so the next
 * paste follows, like the arrangement's clip paste. The playhead stays (pasting is not
 * listening).
 */
export function advanceCursor(at: Beats): void {
  const beats = Math.max(0, at);
  const sel = useTimeSelection.getState().selection;
  if (sel) useTimeSelection.getState().setSelection({ start: beats, end: beats, tracks: sel.tracks });
  else useTimeSelection.setState({ cursor: beats });
}

/** Space: play; while playing, stop and return to the selection, else to where playback started. */
export function playOrStopToStart(transport: EngineTransport): void {
  if (!playing()) {
    transport.send(cmd("Transport", { type: "Play" })).catch(() => {});
    return;
  }
  // Back to the selection (insert marker or section start) when there is one, else to the
  // engine's start position (where the playhead was located before Play).
  const sel = useTimeSelection.getState().selection;
  const start = sel ? sel.start : (useProjectStore.getState().transport?.start_position ?? 0);
  useTimeSelection.setState({ playFrom: null });
  transport.send(cmd("Transport", { type: "Stop" })).catch(() => {});
  locate(transport, start);
}

/** Shift+Space: play; while playing, stop and leave the playhead where it is. */
export function playOrStopHere(transport: EngineTransport): void {
  if (!playing()) {
    transport.send(cmd("Transport", { type: "Play" })).catch(() => {});
    return;
  }
  // Not even to a cursor set while playing (`bindPlayFrom`): the playhead stays.
  useTimeSelection.setState({ playFrom: null });
  transport.send(cmd("Transport", { type: "Stop" })).catch(() => {});
}

/**
 * The Stop button: stop when playing (the playhead stays, or goes to a cursor set while
 * playing); when stopped, return to the selection cursor, or to the project start when
 * already there.
 */
export function stopButton(transport: EngineTransport): void {
  if (playing()) {
    transport.send(cmd("Transport", { type: "Stop" })).catch(() => {});
    return;
  }
  const cursor = selectionCursor();
  locate(transport, Math.abs(playheadBeats() - cursor) > SAME_PLACE ? cursor : 0);
}

/**
 * On stop, locate to the cursor set while playing (so Play starts from it, like Ableton).
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
