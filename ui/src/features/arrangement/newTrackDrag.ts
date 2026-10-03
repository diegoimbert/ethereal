/**
 * Dragging clips below the last track (pure): the empty canvas between the last scrolling
 * track and the pinned master is a "new track" drop zone. Dropped there, the clips land on
 * new tracks, one per source track (MIDI → MIDI track with the built-in synth, like
 * "New track"; audio → audio track), in the source tracks' display order, so a drag across
 * several tracks keeps its relative lanes. Track creation and the move (or copy) are ONE
 * `Edit::Batch`, so one undo step that replicates like any document edit.
 */

import type { Clip, ClipId, Color, Command, TrackId } from "@/generated";
import { addTrackCommand, type NewTrackKind } from "./actions";
import { asOneStep, moveCommand, type ClipBounds } from "./editMath";
import { rowsHeight, type Row } from "./layout";

/** A track a drag below the last track would create (drawn as a ghost lane meanwhile). */
export interface NewLane {
  /** The id the track is created with (fixed for the whole drag). */
  id: TrackId;
  kind: NewTrackKind;
  /** The source track its clips come from (its color tints the ghost clips). */
  source: TrackId;
  color: Color;
}

/**
 * Whether content y (px from the top of the first row) is in the new-track zone: below the
 * last row and, when given, above `visibleBottom` (the bottom of the scrolling area; the
 * pinned master sits under it and is never a drop target).
 */
export function inNewTrackZone(rows: ReadonlyArray<Row>, y: number, visibleBottom = Infinity): boolean {
  return rows.length > 0 && y >= rowsHeight(rows) && y < visibleBottom;
}

/**
 * The new tracks a drag of `clips` below the last track creates: one per distinct source
 * track, in display order (`rows`; tracks not shown keep the clips' order, after them).
 */
export function newTrackLanes(clips: ReadonlyArray<Clip>, rows: ReadonlyArray<Row>, newId: () => string): NewLane[] {
  const order = new Map(rows.map((r, i) => [r.track.id, i]));
  const color = new Map(rows.map((r) => [r.track.id, r.track.color]));
  const sources: Array<{ track: TrackId; kind: NewTrackKind; first: number }> = [];
  clips.forEach((c, i) => {
    if (sources.some((s) => s.track === c.track)) return;
    sources.push({ track: c.track, kind: c.content.type === "Midi" ? "Midi" : "Audio", first: i });
  });
  const rank = (s: (typeof sources)[number]) => order.get(s.track) ?? rows.length + s.first;
  return sources
    .sort((a, b) => rank(a) - rank(b))
    .map((s) => ({ id: newId(), kind: s.kind, source: s.track, color: color.get(s.track) ?? 0x8a91a8 }));
}

/** The drag preview with every clip moved onto its new lane (times unchanged). */
export function onNewLanes(
  preview: ReadonlyMap<ClipId, ClipBounds>,
  clips: ReadonlyArray<Clip>,
  lanes: ReadonlyArray<NewLane>,
): Map<ClipId, ClipBounds> {
  const laneOf = new Map(lanes.map((l) => [l.source, l.id]));
  const out = new Map<ClipId, ClipBounds>();
  for (const c of clips) {
    const p = preview.get(c.id);
    const track = laneOf.get(c.track);
    if (p && track) out.set(c.id, { ...p, track });
  }
  return out;
}

/**
 * The one-step command for a drop below the last track: create the lanes' tracks (in
 * order, after the last regular track), then move or copy the clips onto them (see
 * `moveCommand`; `preview` already targets the lanes, see `onNewLanes`).
 */
export function newTrackDropCommand(
  lanes: ReadonlyArray<NewLane>,
  clips: ReadonlyArray<Clip>,
  preview: ReadonlyMap<ClipId, ClipBounds>,
  copy: boolean,
  newId: () => string,
  all: Iterable<Clip> = clips,
): Command | null {
  const edit = moveCommand(clips, preview, copy, newId, all);
  if (!edit || lanes.length === 0) return edit;
  const commands: Command[] = [];
  for (const lane of lanes) commands.push(...flatten(addTrackCommand(lane.kind, lane.id, newId())));
  commands.push(...flatten(edit));
  const label = `${copy ? "Copy" : "Move"} ${clips.length > 1 ? "Clips" : "Clip"} to New ${lanes.length > 1 ? "Tracks" : "Track"}`;
  return asOneStep(label, commands);
}

function flatten(command: Command): Command[] {
  return command.domain === "Edit" && command.command.type === "Batch" ? command.command.commands : [command];
}
