/**
 * Clip clipboard of the arrangement (and the track clipboard's slot, see `trackEdit.ts`): copy / cut / paste (cmd-C / cmd-X / cmd-V, and the
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

import type { Beats, Clip, ClipId, Command, Note, ProjectId, TrackId, WarpMarker } from "@/generated";
import { notify } from "@/features/notifications";
import { notesOfClip, useProjectStore, warpMarkersOfClip } from "@/state";
import { itemSelection } from "@/timeline";
import { placeInsertMarker } from "@/features/time-edits/marker";
import { useTimeSelection } from "@/features/time-edits/store";
import { cmd, newId, type EngineTransport } from "@/transport";
import { t } from "@/i18n";
import { startOf } from "./clipTime";
import { selectedClips, sendEdit } from "./context";
import { asCountedStep, countedStep, deleteCommand, parkingSpots } from "./editMath";
import { acceptsClip } from "./layout";

interface Entry {
  clip: Clip;
  notes: Note[];
  markers: WarpMarker[];
}

/** `project`: where the clips were copied; they paste only there (#589: another project
 * has none of their tracks or media). */
let clipboard: { project: ProjectId; entries: Entry[]; start: Beats; end: Beats } | null = null;

/**
 * The track clipboard (base-151): copied or cut tracks, top-most first in display order
 * (a group carries its children). See `trackEdit.ts`. `cutUndoId`: the undo step of the
 * cut, while the tracks are deleted (pasting then restores them).
 */
export interface TrackClipboard {
  tracks: TrackId[];
  cutUndoId: number | null;
}
let trackClipboard: TrackClipboard | null = null;

// section-edit: the most recent copy wins ⌘V. A time copy/cut (the time-selection section)
// replaces the clip and track clipboards, and a clip or track copy forgets the others.
useTimeSelection.subscribe((s, prev) => {
  if (s.clipboard && s.clipboard !== prev.clipboard) {
    clipboard = null;
    trackClipboard = null;
  }
});

export function getTrackClipboard(): TrackClipboard | null {
  return trackClipboard;
}

/** Replace the track clipboard (`null` clears it); a track copy forgets the clip and time ones. */
export function setTrackClipboard(next: TrackClipboard | null): void {
  if (next) {
    clipboard = null;
    useTimeSelection.getState().setClipboard(null);
  }
  trackClipboard = next;
}

/** Clips to paste into the open project (copied in it). */
export function hasClipboard(): boolean {
  return clipboard !== null && clipboard.entries.length > 0 && clipboard.project === useProjectStore.getState().project?.id;
}

/** Copy the selected clips. Returns how many were copied. */
export function copyClips(): number {
  const project = useProjectStore.getState().project;
  const clips = selectedClips();
  if (!project || clips.length === 0) return 0;
  clipboard = {
    project: project.id,
    entries: clips.map((clip) => ({ clip, notes: notesOfClip(project, clip.id), markers: warpMarkersOfClip(project, clip.id) })),
    start: Math.min(...clips.map(startOf)),
    end: Math.max(...clips.map((c) => startOf(c) + c.length)),
  };
  trackClipboard = null;
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
  if (clipboard.project !== project.id) {
    notify("Info", t("arrangement.clipboard.other_project"));
    return [];
  }
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
    commands.push(...(project.clips[e.clip.id] ? duplicate(e.clip, id, target, clipStart, park) : rebuild(e, id, target, clipStart)));
    pasted.push(id);
  }
  await sendEdit(transport, asCountedStep(countedStep("undo.clips.paste", pasted.length, "paste clip", "paste clips"), commands));
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
  return [cmd("Clip", { type: "Duplicate", id: clip.id, new_id: id, start: park(clip) }), cmd("Clip", { type: "Move", moves: [{ id, track, start }] })];
}

/** Recreate a clip that is no longer in the project (cut) from its snapshot. */
function rebuild({ clip, notes, markers }: Entry, id: ClipId, track: TrackId, start: Beats): Command[] {
  const c = clip.content;
  const out: Command[] =
    c.type === "Midi"
      ? [cmd("Clip", { type: "CreateMidi", id, track, start, length: clip.length, name: clip.name })]
      : [
          cmd("Clip", { type: "CreateAudio", id, track, start, media: c.media }),
          // Before the bounds: a transpose resizes an unwarped clip (base-144), and a reverse
          // mirrors the bounds (#442; the snapshot's are already on the reversed file).
          cmd("Clip", { type: "SetTranspose", id, semitones: c.transpose }),
          ...(c.reversed ? [cmd("Clip", { type: "SetReversed", id, reversed: true })] : []),
        ];
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
    out.push(cmd("Clip", { type: "SetFades", id, fade_in: c.fade_in, fade_out: c.fade_out }));
    out.push(cmd("Clip", { type: "SetFadeCurves", id, fade_in: c.fade_in_curve, fade_out: c.fade_out_curve }));
    out.push(cmd("Warp", { type: "SetWarp", clip: id, warp: c.warp }));
    for (const m of markers) out.push(cmd("Warp", { type: "AddMarker", id: newId(), clip: id, beat: m.beat, source: m.source }));
  }
  return out;
}

/** Tests: forget the clipboard. */
export function clearClipboard(): void {
  clipboard = null;
  trackClipboard = null;
}
