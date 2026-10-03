/**
 * Clip clipboard of the arrangement: copy / cut / paste (cmd-C / cmd-X / cmd-V, and the
 * right-click menus).
 *
 * Copy snapshots the selected clips (with their notes and warp markers). Paste puts them at
 * a position (the insert marker, else the playhead, or where the lane was right-clicked),
 * keeping their relative timing, as one undo step, then selects the pasted clips and moves
 * the insert marker to their end (never the playhead), so pasting again continues the
 * pattern. A clip still in the
 * project is pasted with `Clip::Duplicate` (exact copy); after a cut, it is rebuilt from
 * the snapshot.
 *
 * Tracks: clips go back to their own tracks; when they all come from one track and
 * `track` is given (the right-clicked lane, or the selected track), they go there instead
 * if it takes that kind of clip.
 */

import type { Beats, Clip, ClipId, Command, Note, TrackId, WarpMarker } from "@/generated";
import { notesOfClip, useProjectStore, warpMarkersOfClip } from "@/state";
import { itemSelection } from "@/timeline";
import { placeInsertMarker } from "@/features/time-edits/marker";
import { useTimeSelection } from "@/features/time-edits/store";
import { cmd, newId, type EngineTransport } from "@/transport";
import { startOf } from "./clipTime";
import { selectedClips, sendEdit } from "./context";
import { asOneStep, deleteCommand, parkingSpots } from "./editMath";
import { acceptsClip } from "./layout";

interface Entry {
  clip: Clip;
  notes: Note[];
  markers: WarpMarker[];
}

let clipboard: { entries: Entry[]; start: Beats; end: Beats } | null = null;

// section-edit: the most recent copy wins ⌘V. A time copy/cut (the time-selection section)
// replaces the clip clipboard, and a clip copy forgets the time one (`copyClips`).
useTimeSelection.subscribe((s, prev) => {
  if (s.clipboard && s.clipboard !== prev.clipboard) clipboard = null;
});

export function hasClipboard(): boolean {
  return clipboard !== null && clipboard.entries.length > 0;
}

/** Copy the selected clips. Returns how many were copied. */
export function copyClips(): number {
  const project = useProjectStore.getState().project;
  const clips = selectedClips();
  if (!project || clips.length === 0) return 0;
  clipboard = {
    entries: clips.map((clip) => ({ clip, notes: notesOfClip(project, clip.id), markers: warpMarkersOfClip(project, clip.id) })),
    start: Math.min(...clips.map(startOf)),
    end: Math.max(...clips.map((c) => startOf(c) + c.length)),
  };
  useTimeSelection.getState().setClipboard(null);
  return clips.length;
}

/** Copy the selected clips, then delete them. */
export async function cutClips(transport: EngineTransport): Promise<void> {
  if (copyClips() === 0) return;
  await sendEdit(transport, deleteCommand(clipboard!.entries.map((e) => e.clip.id)));
}

/** Paste the clipboard with its earliest clip at `at` (see the module doc). */
export async function pasteClips(transport: EngineTransport, at: Beats, track: TrackId | null = null): Promise<ClipId[]> {
  const project = useProjectStore.getState().project;
  if (!project || !clipboard) return [];
  const { entries, start, end } = clipboard;
  const sources = new Set(entries.map((e) => e.clip.track));
  const into = track !== null ? project.tracks[track] : undefined;
  const retarget = into && sources.size === 1 && entries.every((e) => acceptsClip(into, e.clip.content.type)) ? into.id : null;

  const park = parkingSpots(Object.values(project.clips));
  const commands: Command[] = [];
  const pasted: ClipId[] = [];
  for (const e of entries) {
    const target = retarget ?? e.clip.track;
    if (!project.tracks[target]) continue; // its track was deleted
    const id = newId();
    const clipStart = at + (startOf(e.clip) - start);
    commands.push(
      ...(project.clips[e.clip.id] ? duplicate(e.clip, id, target, clipStart, park) : rebuild(e, id, target, clipStart)),
    );
    pasted.push(id);
  }
  await sendEdit(transport, asOneStep(pasted.length > 1 ? "Paste Clips" : "Paste Clip", commands));
  const now = useProjectStore.getState().project;
  const created = pasted.filter((id) => now?.clips[id]);
  if (created.length) itemSelection.getState().select("clip", created, "replace");
  // The insert marker moves to the end (after the selection change, which clears it).
  const tracks = [...new Set(created.map((id) => now!.clips[id]!.track))];
  if (tracks.length) placeInsertMarker(null, at + (end - start), tracks.slice(0, 1));
  return created;
}

function duplicate(clip: Clip, id: ClipId, track: TrackId, start: Beats, park: (c: Clip) => Beats): Command[] {
  if (track === clip.track) return [cmd("Clip", { type: "Duplicate", id: clip.id, new_id: id, start })];
  // Another track: create it clear of the source track's clips first (see `parkingSpots`).
  return [
    cmd("Clip", { type: "Duplicate", id: clip.id, new_id: id, start: park(clip) }),
    cmd("Clip", { type: "Move", moves: [{ id, track, start }] }),
  ];
}

/** Recreate a clip that is no longer in the project (cut) from its snapshot. */
function rebuild({ clip, notes, markers }: Entry, id: ClipId, track: TrackId, start: Beats): Command[] {
  const c = clip.content;
  const out: Command[] =
    c.type === "Midi"
      ? [cmd("Clip", { type: "CreateMidi", id, track, start, length: clip.length, name: clip.name })]
      : [cmd("Clip", { type: "CreateAudio", id, track, start, media: c.media })];
  out.push(cmd("Clip", { type: "SetBounds", id, start, length: clip.length, offset: clip.offset }));
  out.push(cmd("Clip", { type: "SetLoop", id, looping: clip.looping }));
  if (clip.name) out.push(cmd("Clip", { type: "Rename", id, name: clip.name }));
  if (clip.color !== null) out.push(cmd("Clip", { type: "SetColor", id, color: clip.color }));
  if (clip.muted) out.push(cmd("Clip", { type: "SetMuted", ids: [id], muted: true }));
  if (c.type === "Midi") {
    if (notes.length) {
      const specs = notes.map((n) => ({ id: newId(), pitch: n.pitch, velocity: n.velocity, start: n.start, duration: n.duration }));
      out.push(cmd("Note", { type: "Add", clip: id, notes: specs }));
      const muted = specs.filter((_, i) => notes[i]!.muted);
      if (muted.length) {
        const edits = muted.map((s) => ({ id: s.id, pitch: null, velocity: null, start: null, duration: null, muted: true }));
        out.push(cmd("Note", { type: "Edit", edits }));
      }
    }
  } else {
    out.push(cmd("Clip", { type: "SetGain", id, gain: c.gain }));
    out.push(cmd("Clip", { type: "SetTranspose", id, semitones: c.transpose }));
    out.push(cmd("Clip", { type: "SetFades", id, fade_in: c.fade_in, fade_out: c.fade_out }));
    out.push(cmd("Clip", { type: "SetFadeCurves", id, fade_in: c.fade_in_curve, fade_out: c.fade_out_curve }));
    if (c.reversed) out.push(cmd("Clip", { type: "SetReversed", id, reversed: true }));
    out.push(cmd("Warp", { type: "SetWarp", clip: id, warp: c.warp }));
    for (const m of markers) out.push(cmd("Warp", { type: "AddMarker", id: newId(), clip: id, beat: m.beat, source: m.source }));
  }
  return out;
}

/** Tests: forget the clipboard. */
export function clearClipboard(): void {
  clipboard = null;
}
